import XCTest
@testable import IrisDriveIOS

final class DeviceStorageDecodingTests: XCTestCase {
    func testDecodesDeviceStorageRowsAsTheCoreWritesThem() throws {
        let json = #"""
        {
          "summary": "Your Drive is 2.3 GB in 1,204 files. All of it is on this device.",
          "devices": [{
            "pubkey": "aa", "npub": "npub1aa", "name": "MacBook", "is_current": false,
            "app": "macos", "icon": "computer", "has_report": true,
            "held_label": "1.2 GB · 35%", "holds_all": false, "held_permille": 640,
            "connected": false, "connection_label": "Not connected",
            "connection_detail": "Not linked directly to this device now",
            "sync_state": "behind", "sync_label": "Missing recent changes", "sync_detail": "It has not seen 2 changes",
            "space_label": "Uses 2.3 GB of 50.0 GB allowed", "space_detail": "Everything Drive keeps on that device",
            "age_label": "Updated 5 min ago",
            "folders": [
              { "label": "Photos", "depth": 0, "detail": "300.0 MB of 1.2 GB", "held_permille": 250 },
              { "label": "Family", "depth": 1, "detail": "All 100.0 MB", "held_permille": 1000 }
            ],
            "notes": ["Some folders could not be read when it measured."]
          }]
        }
        """#
        let storage = try JSONDecoder().decode(NativeDeviceStorage.self, from: Data(json.utf8))
        XCTAssertEqual(storage.summary, "Your Drive is 2.3 GB in 1,204 files. All of it is on this device.")
        let device = try XCTUnwrap(storage.devices.first)
        XCTAssertEqual(device.id, "aa")
        XCTAssertEqual(device.name, "MacBook")
        XCTAssertFalse(device.isCurrent)
        XCTAssertEqual(device.icon, "computer")
        XCTAssertTrue(device.hasReport)
        XCTAssertEqual(device.heldLabel, "1.2 GB · 35%")
        XCTAssertFalse(device.holdsAll)
        XCTAssertEqual(device.heldPermille, 640)
        XCTAssertEqual(device.connectionLabel, "Not connected")
        XCTAssertEqual(device.syncState, "behind")
        XCTAssertEqual(device.syncLabel, "Missing recent changes")
        XCTAssertEqual(device.spaceLabel, "Uses 2.3 GB of 50.0 GB allowed")
        XCTAssertEqual(device.ageLabel, "Updated 5 min ago")
        XCTAssertEqual(device.folders.map(\.label), ["Photos", "Family"])
        XCTAssertEqual(device.folders.map(\.depth), [0, 1])
        XCTAssertEqual(device.folders.first?.detail, "300.0 MB of 1.2 GB")
        XCTAssertEqual(device.folders.first?.heldPermille, 250)
        XCTAssertEqual(device.notes, ["Some folders could not be read when it measured."])
    }

    func testCoreStateDecodesWithAndWithoutDeviceStorage() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let app = IrisDriveNativeCore(dataDir: dir.appendingPathComponent("config").path, appVersion: "test")
        let json = app.dispatchJson(#"{"type":"create_profile","app_key_label":"Storage test"}"#)

        let state = try JSONDecoder().decode(NativeAppState.self, from: Data(json.utf8))
        let storage = try XCTUnwrap(state.ui.deviceStorage)
        XCTAssertTrue(storage.devices.contains(where: \.isCurrent))

        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        var ui = try XCTUnwrap(object["ui"] as? [String: Any])
        ui.removeValue(forKey: "device_storage")
        object["ui"] = ui
        let olderCore = try JSONSerialization.data(withJSONObject: object)
        let older = try JSONDecoder().decode(NativeAppState.self, from: olderCore)
        XCTAssertNil(older.ui.deviceStorage)
        XCTAssertEqual(older.ui.deviceStorage ?? NativeDeviceStorage(), NativeDeviceStorage())
    }
}
