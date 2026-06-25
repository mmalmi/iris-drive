import Foundation

let foregroundSyncIntervalNanoseconds: UInt64 = 5_000_000_000
let awaitingApprovalForegroundSyncIntervalNanoseconds: UInt64 = 15_000_000_000
let awaitingApprovalScreenRefreshIntervalNanoseconds: UInt64 = 3_000_000_000

enum IrisDriveBackgroundSyncTask {
    static let identifier = "to.iris.drive.ios.background-sync"
}
