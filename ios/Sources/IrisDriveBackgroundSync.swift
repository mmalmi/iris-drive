import Foundation

let foregroundSyncIntervalNanoseconds: UInt64 = 30_000_000_000
let foregroundDriveSyncMinimumIntervalSeconds: TimeInterval = 60
let pendingApprovalReceiptForegroundSyncIntervalNanoseconds: UInt64 = 1_000_000_000
let awaitingApprovalForegroundSyncIntervalNanoseconds: UInt64 = 15_000_000_000
let awaitingApprovalScreenRefreshIntervalNanoseconds: UInt64 = 3_000_000_000

func irisDriveForegroundSyncIsDue(
    elapsed: TimeInterval,
    pendingApprovalReceiptCount: UInt64
) -> Bool {
    pendingApprovalReceiptCount > 0
        || elapsed < 0
        || elapsed >= foregroundDriveSyncMinimumIntervalSeconds
}

func irisDriveForegroundSyncIntervalNanoseconds(
    isAwaitingApproval: Bool,
    pendingApprovalReceiptCount: UInt64
) -> UInt64 {
    if pendingApprovalReceiptCount > 0 {
        return pendingApprovalReceiptForegroundSyncIntervalNanoseconds
    }
    if isAwaitingApproval {
        return awaitingApprovalForegroundSyncIntervalNanoseconds
    }
    return foregroundSyncIntervalNanoseconds
}

func irisDriveShouldRunApprovalAckSync(pendingApprovalReceiptCount: UInt64) -> Bool {
    pendingApprovalReceiptCount > 0
}

func irisDriveShouldAuditPendingReceiptTransition(
    from previousCount: UInt64?,
    to currentCount: UInt64?
) -> Bool {
    guard previousCount != currentCount else { return false }
    return (previousCount ?? 0) > 0 || (currentCount ?? 0) > 0
}

enum IrisDriveBackgroundSyncTask {
    static let identifier = Bundle.main.object(
        forInfoDictionaryKey: "IrisDriveBackgroundSyncTaskIdentifier"
    ) as? String ?? "fi.siriusbusiness.drive.background-sync"
}
