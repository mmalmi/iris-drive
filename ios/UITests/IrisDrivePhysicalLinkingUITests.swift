import CryptoKit
import Foundation
import XCTest

private enum PhysicalLinkMarker {
    private static let fileName = "iris-drive-physical-link-markers.log"

    static func reset(testName: String) {
        try? FileManager.default.removeItem(at: fileURL)
        emit("IRIS_XCUITEST_STARTED=\(testName)")
    }

    static func emit(_ marker: String) {
        let runId = ProcessInfo.processInfo.environment["IRIS_XCUITEST_RUN_ID"] ?? "missing"
        let data = Data("IRIS_XCUITEST_RUN_ID=\(runId)\n\(marker)\n".utf8)
        do {
            try FileManager.default.createDirectory(
                at: fileURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            if FileManager.default.fileExists(atPath: fileURL.path) {
                let handle = try FileHandle(forWritingTo: fileURL)
                try handle.seekToEnd()
                try handle.write(contentsOf: data)
                try handle.close()
            } else {
                try data.write(to: fileURL, options: .atomic)
            }
        } catch {
            XCTFail("Could not write physical-link marker: \(error.localizedDescription)")
        }
        FileHandle.standardError.write(data)
    }

    static func sha256(_ content: String) -> String {
        SHA256.hash(data: Data(content.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private static var fileURL: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(fileName)
    }
}

private enum PhysicalLinkTimeouts {
    static let delivery = bounded(
        environment: "IRIS_XCUITEST_DELIVERY_WAIT_SECS",
        defaultValue: 15,
        maximum: 15
    )
    static let camera = bounded(
        environment: "IRIS_XCUITEST_CAMERA_WAIT_SECS",
        defaultValue: 30,
        maximum: 30
    )
    static let postLink: TimeInterval = 60

    private static func bounded(
        environment name: String,
        defaultValue: TimeInterval,
        maximum: TimeInterval
    ) -> TimeInterval {
        guard let raw = ProcessInfo.processInfo.environment[name],
              let requested = TimeInterval(raw),
              requested > 0
        else {
            return defaultValue
        }
        return min(requested, maximum)
    }
}

/// Physical-device coverage for compact AppKey requests exchanged through the
/// shipped camera and manual-entry interfaces.
final class IrisDrivePhysicalLinkingUITests: XCTestCase {
    private let app = XCUIApplication()

    override func setUpWithError() throws {
        continueAfterFailure = false
        #if targetEnvironment(simulator)
        XCTFail("Physical linking XCTest must run on a real iOS device")
        #endif
        PhysicalLinkMarker.reset(testName: name)
    }

    func testPhysicalEnvironmentBridgeIsReady() {
        XCTAssertEqual(requiredEnvironment("IRIS_XCUITEST_PHYSICAL_LINK_GATE"), "1")
        XCTAssertEqual(requiredEnvironment("IRIS_XCUITEST_ISOLATED_DEVICE"), "1")
        XCTAssertFalse(requiredEnvironment("IRIS_XCUITEST_RUN_ID").isEmpty)
        PhysicalLinkMarker.emit("IRIS_XCUITEST_ENVIRONMENT_READY=1")
        PhysicalLinkMarker.emit("IRIS_XCUITEST_FINISHED=testPhysicalEnvironmentBridgeIsReady")
    }

    func testIosOwnerApprovesAndroidThroughPhysicalCamera() {
        defer {
            PhysicalLinkMarker.emit(
                "IRIS_XCUITEST_FINISHED=testIosOwnerApprovesAndroidThroughPhysicalCamera"
            )
        }
        launchCleanAppGroup()
        createProfile()
        restartAndAssertAuthorized()
        openApprovalScanner()

        let camera = app.descendants(matching: .any)["qrScannerCamera"]
        XCTAssertTrue(camera.waitForExistence(timeout: 10), app.debugDescription)
        PhysicalLinkMarker.emit("IRIS_IOS_CAMERA_READY=1")

        let confirmation = app.alerts["Approve this device?"]
        XCTAssertTrue(
            confirmation.waitForExistence(timeout: PhysicalLinkTimeouts.camera),
            "The physical iOS camera did not decode Android's request QR."
        )
        PhysicalLinkMarker.emit("IRIS_IOS_APPROVAL_CONFIRMATION_READY=1")
        waitForHostSignal("submit-camera-approval")
        confirmation.buttons["Approve"].tap()
        PhysicalLinkMarker.emit("IRIS_IOS_APPROVAL_SUBMITTED=1")

        completePostLinkExchange()
    }

    func testIosJoinerDisplaysPhysicalQrAndBecomesAuthorized() {
        defer {
            PhysicalLinkMarker.emit(
                "IRIS_XCUITEST_FINISHED=testIosJoinerDisplaysPhysicalQrAndBecomesAuthorized"
            )
        }
        launchCleanAppGroup()
        startJoiner(requestReadyMarker: "IRIS_IOS_REQUEST_QR_READY=1")
        waitForJoinerAuthorization(
            marker: "IRIS_IOS_AUTHORIZED=1",
            failure: "iOS stayed on Waiting for approval after Android confirmed the camera scan."
        )

        completePostLinkExchange()
    }

    func testIosOwnerApprovesAndroidThroughManualEntry() {
        defer {
            PhysicalLinkMarker.emit(
                "IRIS_XCUITEST_FINISHED=testIosOwnerApprovesAndroidThroughManualEntry"
            )
        }
        launchCleanAppGroup()
        createProfile()
        restartAndAssertAuthorized()

        let manual = openManualApprovalEntry()
        let request = decodedEnvironment("IRIS_XCUITEST_MANUAL_REQUEST_B64")
        manual.tap()
        manual.typeText(request)
        PhysicalLinkMarker.emit("IRIS_IOS_MANUAL_REQUEST_ENTERED=1")

        let confirmation = app.alerts["Approve this device?"]
        XCTAssertTrue(
            confirmation.waitForExistence(timeout: 10),
            "iOS did not recognize Android's complete manual approval request."
        )
        PhysicalLinkMarker.emit("IRIS_IOS_MANUAL_CONFIRMATION_READY=1")
        waitForHostSignal("submit-manual-approval")
        confirmation.buttons["Approve"].tap()
        PhysicalLinkMarker.emit("IRIS_IOS_MANUAL_APPROVAL_SUBMITTED=1")

        completePostLinkExchange()
    }

    func testIosJoinerWaitsForAndroidManualApproval() {
        defer {
            PhysicalLinkMarker.emit(
                "IRIS_XCUITEST_FINISHED=testIosJoinerWaitsForAndroidManualApproval"
            )
        }
        launchCleanAppGroup()
        startJoiner(requestReadyMarker: "IRIS_IOS_MANUAL_REQUEST_READY=1")
        waitForJoinerAuthorization(
            marker: "IRIS_IOS_MANUAL_AUTHORIZED=1",
            failure: "iOS stayed on Waiting for approval after Android confirmed manual entry."
        )

        completePostLinkExchange()
    }

    private func launchCleanAppGroup() {
        var environment = appEnvironment()
        environment["IRIS_DRIVE_DEBUG_ACTION"] = "reset-local-state"
        app.launchEnvironment = environment
        app.launch()
        app.launchEnvironment = appEnvironment()
    }

    private func restartAndAssertAuthorized() {
        app.terminate()
        app.launchEnvironment = appEnvironment()
        app.launch()
        XCTAssertTrue(tabButton("My Drive").waitForExistence(timeout: 20), app.debugDescription)
        XCTAssertFalse(app.descendants(matching: .any)["awaitingApprovalView"].exists)
        XCUIDevice.shared.press(.home)
        app.activate()
        XCTAssertTrue(tabButton("My Drive").waitForExistence(timeout: 10), app.debugDescription)
        PhysicalLinkMarker.emit("IRIS_IOS_LIFECYCLE_RESUMED=1")
    }

    private func completePostLinkExchange() {
        waitForHostSignal("peer-authorized")
        PhysicalLinkMarker.emit("IRIS_IOS_HOST_AUTHORIZATION_OBSERVED=1")
        restartAndAssertAuthorized()
        sharePostLinkFileThroughSystemExtension()
        PhysicalLinkMarker.emit("IRIS_IOS_POST_LINK_FILE_READY=1")
        waitForHostSignal("peer-file-written")
        assertPeerFileVisibleThroughSystemFiles()
        PhysicalLinkMarker.emit("IRIS_IOS_PEER_FILE_VISIBLE=1")
    }

    private func startJoiner(requestReadyMarker: String) {
        let signIn = app.buttons["welcomeSignIn"]
        XCTAssertTrue(signIn.waitForExistence(timeout: 10), app.debugDescription)
        signIn.tap()

        let requestQr = app.descendants(matching: .any)["approvalRequestQr"]
        XCTAssertTrue(requestQr.waitForExistence(timeout: 15), app.debugDescription)
        XCUIDevice.shared.press(.home)
        app.activate()
        XCTAssertTrue(requestQr.waitForExistence(timeout: 10), app.debugDescription)
        PhysicalLinkMarker.emit("IRIS_IOS_LIFECYCLE_RESUMED=1")
        PhysicalLinkMarker.emit(requestReadyMarker)
    }

    private func waitForJoinerAuthorization(marker: String, failure: String) {
        XCTAssertTrue(
            tabButton("My Drive").waitForExistence(
                timeout: PhysicalLinkTimeouts.camera + PhysicalLinkTimeouts.delivery
            ),
            failure
        )
        XCTAssertFalse(app.descendants(matching: .any)["awaitingApprovalView"].exists)
        PhysicalLinkMarker.emit(marker)
    }

    private func createProfile() {
        let create = app.buttons["welcomeCreateProfile"]
        XCTAssertTrue(create.waitForExistence(timeout: 10), app.debugDescription)
        create.tap()
        let submit = app.buttons["createProfileSubmit"]
        XCTAssertTrue(submit.waitForExistence(timeout: 10), app.debugDescription)
        submit.tap()
        XCTAssertTrue(tabButton("My Drive").waitForExistence(timeout: 20), app.debugDescription)
    }

    private func sharePostLinkFileThroughSystemExtension() {
        let fileName = requiredEnvironment("IRIS_XCUITEST_POST_LINK_FILE")
        let content = decodedEnvironment("IRIS_XCUITEST_POST_LINK_CONTENT_B64")
        let source = XCUIApplication(
            bundleIdentifier: requiredEnvironment("IRIS_XCUITEST_SHARE_SOURCE_BUNDLE_ID")
        )
        source.launchEnvironment["IRIS_DRIVE_SHARE_SOURCE_FILENAME"] = fileName
        source.launchEnvironment["IRIS_DRIVE_SHARE_SOURCE_CONTENT"] = content
        source.launch()

        let share = source.buttons["shareFileToIrisDriveButton"]
        XCTAssertTrue(share.waitForExistence(timeout: 10), source.debugDescription)
        share.tap()
        tapSaveToIrisDrive(sourceApp: source)
        waitForShareSheetToDismiss(sourceApp: source)
        source.terminate()

        app.activate()
        XCTAssertTrue(tabButton("My Drive").waitForExistence(timeout: 15), app.debugDescription)
        XCTAssertTrue(waitForFileCount(atLeast: 1, timeout: 25), app.debugDescription)
        PhysicalLinkMarker.emit("IRIS_IOS_PROVIDER_WRITE_SHA256=\(PhysicalLinkMarker.sha256(content))")
    }

    private func tapSaveToIrisDrive(sourceApp: XCUIApplication) {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let sharing = XCUIApplication(bundleIdentifier: "com.apple.SharingViewService")
        let deadline = Date().addingTimeInterval(25)
        var openedMore = false
        while Date() < deadline {
            for candidate in [sourceApp, springboard, sharing] where candidate.state != .notRunning {
                for element in [
                    candidate.buttons["Save to Iris Drive"].firstMatch,
                    candidate.cells["Save to Iris Drive"].firstMatch,
                    candidate.staticTexts["Save to Iris Drive"].firstMatch,
                ] where element.exists {
                    makeHittable(element, in: candidate)
                    element.tap()
                    return
                }
                if !openedMore {
                    let more = candidate.buttons["More"].firstMatch
                    if more.exists {
                        makeHittable(more, in: candidate)
                        more.tap()
                        openedMore = true
                    }
                }
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTFail("Could not find Save to Iris Drive in the system share sheet")
    }

    private func waitForShareSheetToDismiss(sourceApp: XCUIApplication) {
        let deadline = Date().addingTimeInterval(15)
        while Date() < deadline {
            if sourceApp.buttons["shareFileToIrisDriveButton"].isHittable
                || sourceApp.staticTexts["Saved to Iris Drive"].exists {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTFail("Save to Iris Drive did not complete")
    }

    private func assertPeerFileVisibleThroughSystemFiles() {
        let expected = requiredEnvironment("IRIS_XCUITEST_PEER_FILE")
        XCTAssertTrue(waitForFileCount(atLeast: 2, timeout: PhysicalLinkTimeouts.postLink))
        let open = app.buttons["openInFilesButton"]
        makeHittable(open, in: app)
        open.tap()

        let files = XCUIApplication(bundleIdentifier: "com.apple.DocumentsApp")
        let deadline = Date().addingTimeInterval(45)
        while Date() < deadline {
            if files.state == .runningForeground {
                if files.descendants(matching: .any)[expected].exists
                    || files.descendants(matching: .any)[URL(fileURLWithPath: expected)
                        .deletingPathExtension().lastPathComponent].exists {
                    return
                }
                let drive = files.descendants(matching: .any)["Iris Drive"].firstMatch
                if drive.exists, drive.isHittable { drive.tap() }
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTFail("System Files did not expose the linked peer file \(expected): \(files.debugDescription)")
    }

    private func waitForFileCount(atLeast expected: Int, timeout: TimeInterval) -> Bool {
        let row = app.descendants(matching: .any)["filesSummaryRow"]
        guard row.waitForExistence(timeout: min(timeout, 10)) else { return false }
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            let value = (row.value as? String) ?? row.label
            if Int(value.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0 >= expected {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        return false
    }

    private func openApprovalScanner() {
        _ = openManualApprovalEntry()
        let scan = app.buttons["scanApprovalRequestQr"]
        for _ in 0..<8 where !scan.isHittable { app.swipeUp() }
        XCTAssertTrue(scan.waitForExistence(timeout: 5), app.debugDescription)
        scan.tap()
        allowCameraAccessIfNeeded()
    }

    private func openManualApprovalEntry() -> XCUIElement {
        tabButton("Devices").tap()
        let addDevice = app.buttons["Add Device"]
        XCTAssertTrue(addDevice.waitForExistence(timeout: 10), app.debugDescription)
        addDevice.tap()
        let manual = app.textFields["manualDeviceId"]
        XCTAssertTrue(manual.waitForExistence(timeout: 5), app.debugDescription)
        let manualApprove = app.buttons["Approve"]
        XCTAssertTrue(manualApprove.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(manualApprove.isEnabled)
        PhysicalLinkMarker.emit("IRIS_IOS_MANUAL_ENTRY_READY=1")
        return manual
    }

    private func waitForHostSignal(_ name: String) {
        let runId = requiredEnvironment("IRIS_XCUITEST_RUN_ID")
        let signalPrefix = requiredEnvironment("IRIS_XCUITEST_SIGNAL_PREFIX")
        let signal = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(
                "iris-drive-physical-link-\(runId)-\(signalPrefix)-\(name).signal"
            )
        let deadline = Date().addingTimeInterval(PhysicalLinkTimeouts.postLink)
        while Date() < deadline {
            if FileManager.default.fileExists(atPath: signal.path) { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        }
        XCTFail("Host did not provide physical-link coordination signal \(name)")
    }

    private func appEnvironment() -> [String: String] {
        ["IRIS_DRIVE_ALLOW_DESTRUCTIVE_DEBUG_ACTIONS_ON_DEVICE": "1"]
    }

    private func tabButton(_ title: String) -> XCUIElement {
        app.tabBars.buttons.matching(identifier: title).firstMatch
    }

    private func makeHittable(_ element: XCUIElement, in application: XCUIApplication) {
        for _ in 0..<5 where !element.isHittable { application.swipeUp() }
        XCTAssertTrue(element.exists, element.debugDescription)
        XCTAssertTrue(element.isHittable, element.debugDescription)
    }

    private func allowCameraAccessIfNeeded() {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        for title in ["Allow", "OK"] {
            let button = springboard.alerts.buttons[title]
            if button.waitForExistence(timeout: 3) {
                button.tap()
                return
            }
        }
    }

    private func decodedEnvironment(_ name: String) -> String {
        let encoded = requiredEnvironment(name)
        let padding = String(repeating: "=", count: (4 - encoded.count % 4) % 4)
        let standard = encoded.replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/") + padding
        guard let data = Data(base64Encoded: standard),
              let value = String(data: data, encoding: .utf8)
        else {
            XCTFail("Physical gate provided invalid base64 in \(name)")
            return ""
        }
        return value
    }

    private func requiredEnvironment(_ name: String) -> String {
        let value = ProcessInfo.processInfo.environment[name]?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        XCTAssertFalse(value.isEmpty, "Physical gate did not provide \(name)")
        return value
    }
}
