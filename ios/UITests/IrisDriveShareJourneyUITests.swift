import XCTest

/// Sharing by link and photo backup, tapped through as a person would.
/// Set IRIS_DRIVE_UI_TEST_SCREENSHOT_DIR to keep a screenshot of each step.
final class IrisDriveShareJourneyUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testShareAFolderByLinkThenJoinAndCancelFromALink() throws {
        let app = launchFreshApp()
        tab("Shares", in: app).tap()
        XCTAssertTrue(app.navigationBars["Shares"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["copyShareIdentityButton"].exists)
        keep("01-shares-empty")

        // Sharing a folder goes straight to its link.
        let source = app.textFields["shareSourceInput"]
        XCTAssertTrue(source.waitForExistence(timeout: 10))
        source.tap()
        source.typeText("Trip")
        app.buttons["createShareButton"].tap()
        let copy = app.buttons["shareLinkCopy"]
        XCTAssertTrue(copy.waitForExistence(timeout: 30), app.debugDescription)
        XCTAssertTrue(app.buttons["shareLinkSend"].exists)
        keep("02-share-sheet-view-link")
        // The link itself is covered by the Rust and cross-app tests; reading the
        // pasteboard here would stop on the system's paste prompt.
        XCTAssertTrue(copy.isEnabled)
        app.segmentedControls.buttons["Edit"].tap()
        XCTAssertTrue(copy.waitForExistence(timeout: 10))
        XCTAssertTrue(copy.isEnabled)
        keep("03-share-sheet-edit-link")
        copy.tap()
        app.buttons["Done"].tap()

        // The owner's folder is listed once, where it sits in My Drive.
        let row = app.staticTexts["Trip"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'In My Drive'")).firstMatch.exists)
        row.tap()
        keep("04-owned-share-row")
        let more = app.buttons["shareMoreMenu"]
        XCTAssertTrue(more.waitForExistence(timeout: 5))
        more.tap()
        XCTAssertTrue(app.buttons["Stop Sharing…"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Remove from My Drive"].exists)
        keep("05-owned-share-menu")
        app.buttons["Stop Sharing…"].tap()
        XCTAssertTrue(app.alerts["Stop sharing?"].waitForExistence(timeout: 5))
        keep("06-stop-sharing-confirm")
        app.alerts["Stop sharing?"].buttons["Cancel"].tap()

        // Someone else's link: ask to join, see it waiting, then stop waiting.
        let join = app.textFields["shareInviteInput"]
        scrollTo(join, in: app)
        join.tap()
        join.typeText(Self.sampleJoinLink)
        app.buttons["acceptShareInviteButton"].tap()
        let waiting = app.descendants(matching: .any)["pendingShareJoin"].firstMatch
        XCTAssertTrue(waiting.waitForExistence(timeout: 30), app.debugDescription)
        XCTAssertTrue(app.staticTexts["Waiting to join “Book club”"].exists)
        keep("07-waiting-to-join")
        app.staticTexts["Waiting to join “Book club”"].swipeLeft()
        app.buttons["Cancel"].tap()
        XCTAssertFalse(app.staticTexts["Waiting to join “Book club”"].waitForExistence(timeout: 3))
        keep("08-join-cancelled")
    }

    func testPhotoBackupShowsItsProgressAndLastBackup() throws {
        let app = launchFreshApp()
        tab("Backup", in: app).tap()
        let toggle = app.switches["photoBackupToggle"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 10), app.debugDescription)
        keep("10-photo-backup-off")
        addUIInterruptionMonitor(withDescription: "Photos access") { alert in
            for label in ["Allow Full Access", "Allow Access to All Photos", "OK"] where alert.buttons[label].exists {
                alert.buttons[label].tap()
                return true
            }
            return false
        }
        toggle.switches.firstMatch.exists ? toggle.switches.firstMatch.tap() : toggle.tap()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.buttons["Allow Full Access"]
        if allow.waitForExistence(timeout: 8) { allow.tap() }
        app.tap()

        let status = app.descendants(matching: .any)["photoBackupStatus"].firstMatch
        XCTAssertTrue(status.waitForExistence(timeout: 30), app.debugDescription)
        keep("11-photo-backup-started")
        // The sample library is small: it finishes, or says plainly what it waits for.
        let settled = NSPredicate(format: "label CONTAINS 'Up to date' OR label CONTAINS 'waiting to sync' OR label CONTAINS 'pending' OR label CONTAINS 'Wi-Fi'")
        let done = app.staticTexts.matching(settled).firstMatch
        XCTAssertTrue(done.waitForExistence(timeout: 180), app.debugDescription)
        keep("12-photo-backup-settled")
    }

    // MARK: Helpers

    /// A join link with a throwaway key for a folder nobody answers for.
    private static let sampleJoinLink: String = {
        let link: [String: Any] = [
            "v": 1,
            "share_id": "11111111-1111-4111-8111-111111111111",
            "owner_profile_id": "22222222-2222-4222-8222-222222222222",
            "admin": String(repeating: "a", count: 64),
            "name": "Book club",
            "role": "reader",
            "key": String(repeating: "01", count: 32),
        ]
        let json = try! JSONSerialization.data(withJSONObject: link, options: [.sortedKeys])
        let payload = json.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
        return "https://drive.iris.to/#/share-join/\(payload)"
    }()

    private func launchFreshApp() -> XCUIApplication {
        let app = XCUIApplication()
        for (key, value) in ProcessInfo.processInfo.environment where key.hasPrefix("IRIS_DRIVE_UI_TEST_") {
            app.launchEnvironment[key] = value
        }
        app.launchEnvironment["IRIS_DRIVE_DEBUG_ACTION"] = "reset-and-seed-provider-file"
        app.launchEnvironment["IRIS_DRIVE_DEBUG_PROVIDER_FILE_NAME"] = "Notes.txt"
        app.launch()
        XCTAssertTrue(tab("My Drive", in: app).waitForExistence(timeout: 40), app.debugDescription)
        return app
    }

    private func tab(_ title: String, in app: XCUIApplication) -> XCUIElement {
        app.tabBars.buttons.matching(identifier: title).firstMatch
    }

    private func scrollTo(_ element: XCUIElement, in app: XCUIApplication) {
        for _ in 0..<6 where !element.isHittable {
            app.swipeDown()
        }
        XCTAssertTrue(element.waitForExistence(timeout: 5))
    }

    private func keep(_ name: String) {
        let shot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: shot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        guard let dir = ProcessInfo.processInfo.environment["IRIS_DRIVE_UI_TEST_SCREENSHOT_DIR"], !dir.isEmpty else {
            return
        }
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        try? shot.pngRepresentation.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name).png"))
    }
}
