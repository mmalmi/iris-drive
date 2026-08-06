import XCTest

final class IrisDriveDeviceApprovalUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testDeviceApprovalUniversalLinkCancelDoesNotApprove() throws {
        let request = try requiredEnvironment("IRIS_DRIVE_UI_TEST_DEEP_LINK_REQUEST")
        let linkedDeviceLabel = try requiredEnvironment("IRIS_DRIVE_UI_TEST_LINKED_DEVICE_LABEL")
        let app = launchApp()
        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 15))

        let openedAt = Date()
        app.open(try XCTUnwrap(URL(string: request)))
        let alert = app.alerts["Approve this device?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertLessThan(Date().timeIntervalSince(openedAt), 15)
        XCTAssertFalse(app.staticTexts[linkedDeviceLabel].exists)

        alert.buttons["Cancel"].tap()
        tabButton("Devices", in: app).tap()
        RunLoop.current.run(until: Date().addingTimeInterval(1))
        XCTAssertFalse(app.staticTexts[linkedDeviceLabel].exists)
    }

    func testDeviceApprovalUniversalLinkApprovesOnlyAfterTap() throws {
        let request = try requiredEnvironment("IRIS_DRIVE_UI_TEST_DEEP_LINK_REQUEST")
        let linkedDeviceLabel = try requiredEnvironment("IRIS_DRIVE_UI_TEST_LINKED_DEVICE_LABEL")
        let app = launchApp()
        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 15))

        app.open(try XCTUnwrap(URL(string: request)))
        let alert = app.alerts["Approve this device?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertFalse(app.staticTexts[linkedDeviceLabel].exists)

        let approvedAt = Date()
        alert.buttons["Approve"].tap()
        tabButton("Devices", in: app).tap()
        XCTAssertTrue(app.staticTexts[linkedDeviceLabel].waitForExistence(timeout: 15))
        XCTAssertLessThan(Date().timeIntervalSince(approvedAt), 15)
    }

    private func launchApp() -> XCUIApplication {
        let app = XCUIApplication()
        for (key, value) in ProcessInfo.processInfo.environment where key.hasPrefix("IRIS_DRIVE_UI_TEST_") {
            app.launchEnvironment[key] = value
        }
        app.launch()
        return app
    }

    private func tabButton(_ title: String, in app: XCUIApplication) -> XCUIElement {
        app.tabBars.buttons.matching(identifier: title).firstMatch
    }

    private func requiredEnvironment(_ name: String) throws -> String {
        let environment = ProcessInfo.processInfo.environment
        let decoded = environment["\(name)_B64"]
            .flatMap { Data(base64Encoded: $0) }
            .flatMap { String(data: $0, encoding: .utf8) }
        let value = environment[name] ?? decoded ?? ""
        if value.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines).isEmpty {
            throw XCTSkip("\(name) is required for this UI test")
        }
        return value
    }
}
