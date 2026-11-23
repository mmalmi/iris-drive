import CryptoKit
import XCTest
@testable import IrisDriveIOS

final class PhysicalLinkedFileTests: XCTestCase {
    func testReadLinkedFileFromDeviceProvider() throws {
        let env = ProcessInfo.processInfo.environment
        try XCTSkipUnless(env["IRIS_XCUITEST_ISOLATED_DEVICE"] == "1",
                          "The physical linking test supplies a reserved app group and file.")
        #if targetEnvironment(simulator)
        XCTFail("Expected the physical device that completed the linking test")
        return
        #else
        let expectedBundle = try XCTUnwrap(env["IRIS_XCUITEST_PROVIDER_BUNDLE"])
        let group = try XCTUnwrap(env["IRIS_XCUITEST_PROVIDER_GROUP"])
        guard Bundle.main.bundleIdentifier == expectedBundle,
              IrisDriveSharedContainer.appGroupIdentifier == group else {
            return XCTFail("The installed app or group differs from the linking test")
        }
        let filename = try XCTUnwrap(env["IRIS_XCUITEST_PEER_FILE"])
        guard filename.range(of: #"^android-(manual-)?(owner|joiner)-[0-9-]+\.txt$"#,
                             options: .regularExpression) != nil else {
            return XCTFail("Expected the file created by the paired Android fixture")
        }
        let expected = try XCTUnwrap(env["IRIS_XCUITEST_PEER_SHA256"])
        let base = try XCTUnwrap(FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group))
            .appendingPathComponent(IrisDriveSharedContainer.storageDirectoryName)
        let restarted = IrisDriveNativeCore(dataDir: base.path, appVersion: "physical-test")
        let state = try JSONDecoder().decode(NativeAppState.self, from: Data(restarted.stateJson().utf8))
        XCTAssertEqual(state.ui.profile?.authorizationState, "authorized")
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: file) }
        _ = IrisDriveNativeProvider.read(dataDir: base.path, path: filename, outputPath: file.path)
        let actual = SHA256.hash(data: try Data(contentsOf: file)).map { String(format: "%02x", $0) }.joined()
        XCTAssertEqual(actual, expected)
        #endif
    }
}
