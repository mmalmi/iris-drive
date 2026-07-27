import XCTest

final class IrisDriveAppStoreScreenshotTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testCaptureAppStoreScreenshots() throws {
        let baseDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-app-store-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: baseDir, withIntermediateDirectories: true)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: baseDir)
        }

        let app = XCUIApplication()
        app.launchEnvironment["IRIS_DRIVE_UI_TEST_BASE_DIR"] = baseDir.path
        app.launchEnvironment["IRIS_DRIVE_UI_TEST_SCREENSHOT_FIXTURE"] = "1"
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()

        let create = app.buttons["welcomeCreateProfile"]
        XCTAssertTrue(create.waitForExistence(timeout: 20), app.debugDescription)
        create.tap()
        let submit = app.buttons["createProfileSubmit"]
        XCTAssertTrue(submit.waitForExistence(timeout: 10), app.debugDescription)
        submit.tap()

        let myDrive = tabButton("My Drive", in: app)
        XCTAssertTrue(myDrive.waitForExistence(timeout: 30), app.debugDescription)
        myDrive.tap()
        XCTAssertTrue(app.staticTexts["Up to date"].waitForExistence(timeout: 15))
        settleAndCapture("01-my-drive")

        let devices = tabButton("Devices", in: app)
        devices.tap()
        XCTAssertTrue(app.staticTexts["Home server"].waitForExistence(timeout: 10))
        settleAndCapture("02-devices")

        let shares = tabButton("Shares", in: app)
        shares.tap()
        XCTAssertTrue(app.staticTexts["Family Photos"].waitForExistence(timeout: 10))
        settleAndCapture("03-shares")

        let backup = tabButton("Backup", in: app)
        backup.tap()
        XCTAssertTrue(app.staticTexts["Iris Archive"].waitForExistence(timeout: 10))
        settleAndCapture("04-backup")
    }

    private func tabButton(_ title: String, in app: XCUIApplication) -> XCUIElement {
        app.buttons[title].firstMatch
    }

    private func settleAndCapture(_ name: String) {
        RunLoop.current.run(until: Date().addingTimeInterval(0.8))
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "screenshot-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
