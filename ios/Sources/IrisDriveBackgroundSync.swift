import Foundation

let foregroundSyncIntervalNanoseconds: UInt64 = 5_000_000_000
let nativeBackgroundStackSize = 8 * 1024 * 1024

enum IrisDriveBackgroundSyncTask {
    static let identifier = "to.iris.drive.ios.background-sync"
}
