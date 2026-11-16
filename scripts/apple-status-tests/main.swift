import Foundation

precondition(IrisDriveDeviceModel.label(identifier: "iPhone18,3", family: "iPhone") == "iPhone 17")
precondition(IrisDriveDeviceModel.label(identifier: "iPhone18,1", family: "iPhone") == "iPhone 17 Pro")
precondition(IrisDriveDeviceModel.label(identifier: "iPhone99,1", family: "iPhone") == "iPhone")
precondition(IrisDriveDeviceModel.label(identifier: "", family: "iPad") == "iPad")

let checked: [String: Any] = ["verified": true, "tag": "v0.1.43", "available": false, "asset": "Drive.app.tar.gz"]
let check = try IrisDriveUpdateCheck(checked)
precondition(check.statusText == "Newest release seen: v0.1.43")
var newer = checked
newer["available"] = true
let update = try IrisDriveUpdateCheck(newer)
precondition(update.statusText == "Update v0.1.43 available")
for key in ["verified", "available", "tag"] {
    var partial = checked
    partial.removeValue(forKey: key)
    precondition((try? IrisDriveUpdateCheck(partial)) == nil)
}
var unverified = checked
unverified["verified"] = false
precondition((try? IrisDriveUpdateCheck(unverified)) == nil)
let failed = ["error": "No current peer observation"]
do {
    _ = try IrisDriveUpdateCheck(failed)
    preconditionFailure("An inconclusive check cannot report a release")
} catch {
    precondition(error.localizedDescription == "No current peer observation")
}
print("APPLE_STATUS_TESTS_OK")
