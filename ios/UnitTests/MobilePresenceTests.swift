import XCTest
@testable import IrisDriveIOS

final class MobilePresenceTests: XCTestCase {
    func testUnavailableRosterRelayCannotLeaveDeviceStatusStale() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let core = IrisDriveNativeCore(dataDir: directory.path, appVersion: "test")
        _ = core.dispatchJson(#"{"type":"replace_relays","urls":["ws://127.0.0.1:1"]}"#)
        _ = core.dispatchJson(#"{"type":"replace_blossom_servers","urls":["http://127.0.0.1:1"]}"#)
        _ = core.dispatchJson(#"{"type":"create_profile","app_key_label":"Presence test"}"#)

        let statusURL = directory.appendingPathComponent("native-fips-status.json")
        func status() -> [String: Any]? {
            guard let data = try? Data(contentsOf: statusURL) else { return nil }
            return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        }
        func freshStatus() -> XCTestExpectation {
            XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                let value = status()
                return value?["running"] as? Bool == true && (value?["updated_at"] as? Int ?? 0) > 0
            }, object: nil)
        }

        XCTAssertEqual(XCTWaiter.wait(for: [freshStatus()], timeout: 30), .completed)
        var old = try XCTUnwrap(status())
        // Model the stale heartbeat left after iOS suspended this process.
        old["updated_at"] = 0
        try JSONSerialization.data(withJSONObject: old).write(to: statusURL, options: .atomic)
        XCTAssertEqual(XCTWaiter.wait(for: [freshStatus()], timeout: 35), .completed)
        withExtendedLifetime(core) {}
    }
}
