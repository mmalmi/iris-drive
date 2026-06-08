import XCTest

final class IrisDriveIOSUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testWelcomeRoutesWithoutSetupTitle() throws {
        let app = launchApp()
        XCTAssertFalse(app.navigationBars["Setup"].exists)
        XCTAssertTrue(app.buttons["welcomeCreateProfile"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["welcomeSignIn"].waitForExistence(timeout: 5))

        app.buttons["welcomeCreateProfile"].tap()
        XCTAssertTrue(app.navigationBars["Create profile"].waitForExistence(timeout: 5))

        app.terminate()
        app.launch()
        app.buttons["welcomeSignIn"].tap()
        XCTAssertTrue(app.navigationBars["Restore"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["openRecoveryPhrase"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["openSecretKey"].waitForExistence(timeout: 5))
        app.buttons["openLinkDevice"].tap()
        XCTAssertTrue(app.navigationBars["Link app install"].waitForExistence(timeout: 5))
    }

    func testLinkThisDeviceFromWelcome() throws {
        let invite = try requiredEnvironment("IRIS_DRIVE_UI_TEST_OWNER_INVITE")
        let app = launchApp()

        app.buttons["welcomeSignIn"].tap()
        app.buttons["openLinkDevice"].tap()

        let owner = app.textFields["linkOwnerInput"]
        if owner.waitForExistence(timeout: 2) {
            XCTAssertEqual(owner.value as? String, invite)
        }

        XCTAssertTrue(
            app.descendants(matching: .any)["awaitingApprovalView"].waitForExistence(timeout: 15)
        )
    }

    func testCreateProfileFromWelcome() throws {
        let app = launchApp()

        app.buttons["welcomeCreateProfile"].tap()
        app.buttons["createProfileSubmit"].tap()

        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 15))
    }

    func testOpenDriveFolderInFilesApp() throws {
        let app = launchApp()
        ensureMyDriveReady(in: app)

        let openInFiles = app.buttons["openInFilesButton"]
        makeHittable(openInFiles, in: app)
        openInFiles.tap()

        let files = XCUIApplication(bundleIdentifier: "com.apple.DocumentsApp")
        assertFilesOpen(in: app, files: files, timeout: 15)
    }

    func testMyDriveDevicesSummaryOpensDevices() throws {
        let app = launchApp()
        ensureMyDriveReady(in: app)

        let devicesSummary = app.buttons["devicesSummaryButton"]
        XCTAssertTrue(devicesSummary.waitForExistence(timeout: 10))
        let value = devicesSummary.value as? String ?? ""
        XCTAssertTrue(value.contains("/"), "unexpected devices summary: \(value)")
        XCTAssertTrue(value.contains(" online"), "unexpected devices summary: \(value)")

        devicesSummary.tap()
        XCTAssertTrue(app.navigationBars["AppKeys"].waitForExistence(timeout: 10))
        XCTAssertTrue(tabButton("AppKeys", in: app).isSelected)
    }

    func testSharesTabExposesSharingView() throws {
        let app = launchApp()
        ensureMyDriveReady(in: app)

        let sharesTab = tabButton("Shares", in: app)
        XCTAssertTrue(sharesTab.waitForExistence(timeout: 10))
        sharesTab.tap()

        XCTAssertTrue(app.navigationBars["Shares"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.textFields["shareSourceInput"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["createShareButton"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.textFields["shareInviteInput"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["acceptShareInviteButton"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["copyShareIdentityButton"].waitForExistence(timeout: 10))
    }

    func testMyDriveFileCountMatchesExpected() throws {
        let expected = try requiredEnvironment("IRIS_DRIVE_UI_TEST_EXPECTED_FILE_COUNT")
        let app = launchApp()
        ensureMyDriveReady(in: app)

        let row = app.descendants(matching: .any)["filesSummaryRow"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        let deadline = Date().addingTimeInterval(45)
        var actual = accessibilityValue(row)
        while Date() < deadline, actual != expected {
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
            actual = accessibilityValue(row)
        }
        XCTAssertEqual(actual, expected, "Files row: \(row.debugDescription)")
    }

    func testApprovedLinkedDeviceLeavesWaiting() throws {
        let app = launchApp()

        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 45))
        XCTAssertFalse(app.descendants(matching: .any)["awaitingApprovalView"].exists)
        XCTAssertTrue(tabButton("AppKeys", in: app).waitForExistence(timeout: 10))
        tabButton("AppKeys", in: app).tap()
        XCTAssertTrue(app.staticTexts["This AppKey"].waitForExistence(timeout: 10))
        XCTAssertTrue(
            waitForLinkedOnlineDeviceRow(in: app, timeout: 10),
            "Expected a linked online device row. Static texts:\n\(staticTextLabels(in: app))"
        )
        XCTAssertFalse(app.staticTexts["Authorized"].exists)
    }

    func testAddLinkedDeviceFromDevices() throws {
        let linkedDevice = try requiredEnvironment("IRIS_DRIVE_UI_TEST_LINKED_DEVICE")
        let app = launchApp()

        XCTAssertTrue(tabButton("AppKeys", in: app).waitForExistence(timeout: 10))
        tabButton("AppKeys", in: app).tap()
        XCTAssertTrue(app.buttons["addDeviceButton"].waitForExistence(timeout: 10))
        app.buttons["addDeviceButton"].tap()

        let deviceField = app.textFields["manualDeviceId"]
        makeHittable(deviceField, in: app)
        XCTAssertEqual(deviceField.value as? String, linkedDevice)

        let nameField = app.textFields["manualDeviceName"]
        makeHittable(nameField, in: app)
        XCTAssertEqual(nameField.value as? String, "iOS UI linked")
        app.buttons["manualDeviceAdd"].tap()

        XCTAssertTrue(app.staticTexts["iOS UI linked"].waitForExistence(timeout: 15))
    }

    private func launchApp() -> XCUIApplication {
        let app = XCUIApplication()
        for (key, value) in ProcessInfo.processInfo.environment
            where key.hasPrefix("IRIS_DRIVE_UI_TEST_") || key.hasPrefix("IRIS_DRIVE_FIPS_") {
            app.launchEnvironment[key] = value
        }
        app.launch()
        return app
    }

    private func ensureMyDriveReady(in app: XCUIApplication) {
        if app.buttons["welcomeCreateProfile"].waitForExistence(timeout: 3) {
            app.buttons["welcomeCreateProfile"].tap()
            app.buttons["createProfileSubmit"].tap()
        }
        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 15))
        tabButton("My Drive", in: app).tap()
    }

    private func tabButton(_ title: String, in app: XCUIApplication) -> XCUIElement {
        app.tabBars.buttons.matching(identifier: title).firstMatch
    }

    private func assertFilesOpen(
        in app: XCUIApplication,
        files: XCUIApplication,
        timeout: TimeInterval
    ) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if files.state == .runningForeground,
               files.descendants(matching: .any)["Iris Drive"].exists {
                return
            }
            if app.state == .runningForeground {
                let error = app.staticTexts["openInFilesError"]
                if error.exists {
                    XCTFail("Open in Files failed: \(error.label)")
                    return
                }
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTFail("Files did not show Iris Drive. Files hierarchy:\n\(files.debugDescription)")
    }

    private func requiredEnvironment(_ name: String) throws -> String {
        let value = ProcessInfo.processInfo.environment[name] ?? ""
        if value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            throw XCTSkip("\(name) is required for this UI test")
        }
        return value
    }

    private func makeHittable(_ element: XCUIElement, in app: XCUIApplication) {
        for _ in 0..<6 where !element.isHittable {
            app.swipeUp()
        }
        XCTAssertTrue(element.waitForExistence(timeout: 2))
        XCTAssertTrue(element.isHittable)
    }

    private func accessibilityValue(_ element: XCUIElement) -> String {
        (element.value as? String ?? element.label).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func waitForLinkedOnlineDeviceRow(in app: XCUIApplication, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hasLinkedOnlineDeviceRow(in: app) {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        }
        return hasLinkedOnlineDeviceRow(in: app)
    }

    private func hasLinkedOnlineDeviceRow(in app: XCUIApplication) -> Bool {
        app.staticTexts.allElementsBoundByIndex.contains { element in
            let label = element.label
            return label.hasPrefix("Member | Linked | Online")
                || label.hasPrefix("Admin | Linked | Online")
        }
    }

    private func staticTextLabels(in app: XCUIApplication) -> String {
        app.staticTexts.allElementsBoundByIndex
            .map(\.label)
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }
}
