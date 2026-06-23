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
        XCTAssertTrue(app.navigationBars["Link device"].waitForExistence(timeout: 5))
    }

    func testLinkThisDeviceFromWelcome() throws {
        let invite = try requiredEnvironment("IRIS_DRIVE_UI_TEST_OWNER_INVITE")
        let app = launchApp()

        app.buttons["welcomeSignIn"].tap()
        app.buttons["openLinkDevice"].tap()

        let awaitingApproval = app.descendants(matching: .any)["awaitingApprovalView"]
        let deadline = Date().addingTimeInterval(30)
        var checkedPrefilledTarget = false
        while Date() < deadline {
            if awaitingApproval.exists {
                return
            }
            let owner = app.textFields["linkTargetInput"]
            if owner.exists, !checkedPrefilledTarget {
                XCTAssertTrue(accessibilityValue(owner).contains("drive.iris"), invite)
                checkedPrefilledTarget = true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTFail(app.debugDescription)
    }

    func testCreateProfileFromWelcome() throws {
        let app = launchApp()

        app.buttons["welcomeCreateProfile"].tap()
        app.buttons["createProfileSubmit"].tap()

        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 15))
    }

    func testDebugResetLocalStateReturnsToWelcome() throws {
        let baseDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-ui-test-reset-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: baseDir, withIntermediateDirectories: true)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: baseDir)
        }
        let app = launchApp(environment: [
            "IRIS_DRIVE_UI_TEST_BASE_DIR": baseDir.path,
        ])

        app.buttons["welcomeCreateProfile"].tap()
        app.buttons["createProfileSubmit"].tap()
        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 15))

        app.terminate()
        let resetApp = launchApp(environment: [
            "IRIS_DRIVE_UI_TEST_BASE_DIR": baseDir.path,
            "IRIS_DRIVE_DEBUG_ACTION": "reset-local-state",
        ])

        XCTAssertTrue(resetApp.buttons["welcomeCreateProfile"].waitForExistence(timeout: 5))
        XCTAssertTrue(resetApp.buttons["welcomeSignIn"].waitForExistence(timeout: 5))
    }

    func testCreateProfileSurfacesNativeSetupError() throws {
        let blockedBaseDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-ui-test-blocked-\(UUID().uuidString)")
        try "not a directory".write(to: blockedBaseDir, atomically: true, encoding: .utf8)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: blockedBaseDir)
        }
        let app = launchApp(environment: [
            "IRIS_DRIVE_UI_TEST_BASE_DIR": blockedBaseDir.path,
        ])

        app.buttons["welcomeCreateProfile"].tap()
        app.buttons["createProfileSubmit"].tap()

        let error = app.staticTexts["setupErrorMessage"]
        XCTAssertTrue(error.waitForExistence(timeout: 5))
        XCTAssertTrue(accessibilityValue(error).contains("creating profile"))
    }

    func testOpenDriveFolderInFilesApp() throws {
        let seededFile = "Iris Drive UI provider entry.txt"
        let app = launchApp(environment: [
            "IRIS_DRIVE_DEBUG_ACTION": "reset-and-seed-provider-file",
            "IRIS_DRIVE_DEBUG_PROVIDER_FILE_NAME": seededFile,
            "IRIS_DRIVE_DEBUG_PROVIDER_FILE_CONTENT": "Files enumeration test\n",
        ])
        ensureMyDriveReady(in: app)
        let row = app.descendants(matching: .any)["filesSummaryRow"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        XCTAssertTrue(
            waitForValue(row, containing: "1", timeout: 15),
            "Expected seeded provider file count before opening Files. Row: \(row.debugDescription)"
        )

        let openInFiles = app.buttons["openInFilesButton"]
        makeHittable(openInFiles, in: app)
        openInFiles.tap()

        let files = XCUIApplication(bundleIdentifier: "com.apple.DocumentsApp")
        assertFilesOpen(in: app, files: files, timeout: 20, expectedItem: seededFile)
    }

    func testOpenIrisAppsLoadsBrowserWithoutConnectionError() throws {
        let app = launchApp()
        ensureMyDriveReady(in: app)

        assertOpenIrisAppsLoads(in: app)
    }

    func testOpenIrisAppsLoadsBrowserWhenSyncPaused() throws {
        let app = launchApp()
        ensureMyDriveReady(in: app)

        let pauseSync = app.buttons["Pause sync"].firstMatch
        makeHittable(pauseSync, in: app)
        pauseSync.tap()
        XCTAssertTrue(app.buttons["Resume sync"].waitForExistence(timeout: 5), app.debugDescription)
        app.swipeDown()

        assertOpenIrisAppsLoads(in: app)
    }

    func testIrisWebFooterBrowserStyleScreenshots() throws {
        let screenshotDir = optionalEnvironment("IRIS_DRIVE_UI_SCREENSHOT_DIR")
            ?? FileManager.default.temporaryDirectory
                .appendingPathComponent("iris-drive-browser-footer-shots")
                .path
        let browserURL = try optionalEnvironment("IRIS_DRIVE_UI_TEST_BROWSER_URL")
            ?? localBrowserFooterFixtureURL()
        let debugAction = optionalEnvironment("IRIS_DRIVE_UI_TEST_BROWSER_DEBUG_ACTION") ?? "open-browser"
        let app = launchApp(environment: [
            "IRIS_DRIVE_DEBUG_ACTION": debugAction,
            "IRIS_DRIVE_DEBUG_BROWSER_URL": browserURL,
        ])

        let address = app.descendants(matching: .any)["irisWebAddressField"]
        XCTAssertTrue(address.waitForExistence(timeout: 35), app.debugDescription)
        waitForIrisBrowserToFinishLoading(in: app)
        XCTAssertTrue(app.buttons["irisWebReloadButton"].exists, app.debugDescription)
        XCTAssertTrue(app.buttons["irisWebMoreButton"].exists, app.debugDescription)
        if let expectedTitle = optionalEnvironment("IRIS_DRIVE_UI_TEST_BROWSER_TITLE_CONTAINS") {
            XCTAssertTrue(
                waitForValue(address, containing: expectedTitle, timeout: 12),
                "Expected browser footer title to contain \(expectedTitle), got \(accessibilityValue(address))"
            )
        }
        try saveScreenshot(named: "iris-web-footer-expanded", in: screenshotDir)

        app.swipeUp()
        app.swipeUp()
        let compactTitle = app.buttons["irisWebCompactTitle"].firstMatch
        XCTAssertTrue(compactTitle.waitForExistence(timeout: 5), app.debugDescription)
        try saveScreenshot(named: "iris-web-footer-collapsed", in: screenshotDir)

        compactTitle.tap()
        XCTAssertTrue(address.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["irisWebReloadButton"].exists, app.debugDescription)
        XCTAssertTrue(app.buttons["irisWebMoreButton"].exists, app.debugDescription)
        try saveScreenshot(named: "iris-web-footer-reexpanded", in: screenshotDir)
    }

    func testIrisWebAddressFieldFocusShowsFullURL() throws {
        let browserURL = try optionalEnvironment("IRIS_DRIVE_UI_TEST_BROWSER_URL")
            ?? localBrowserFooterFixtureURL()
        let debugAction = optionalEnvironment("IRIS_DRIVE_UI_TEST_BROWSER_DEBUG_ACTION") ?? "open-browser"
        let app = launchApp(environment: [
            "IRIS_DRIVE_DEBUG_ACTION": debugAction,
            "IRIS_DRIVE_DEBUG_BROWSER_URL": browserURL,
        ])

        let addressBar = app.descendants(matching: .any)["irisWebAddressField"]
        XCTAssertTrue(addressBar.waitForExistence(timeout: 35), app.debugDescription)
        waitForIrisBrowserToFinishLoading(in: app)

        XCTAssertTrue(app.buttons["irisWebAddressField"].waitForExistence(timeout: 5), app.debugDescription)
        app.buttons["irisWebAddressField"].tap()

        let focusedAddress = app.textFields["irisWebAddressField"]
        XCTAssertTrue(
            focusedAddress.waitForExistence(timeout: 5),
            "Expected the address control to become an editable text field"
        )
        let expectedAddressFragment = browserURL
            .replacingOccurrences(of: #"^https?:/+"#, with: "", options: .regularExpression)
        XCTAssertTrue(
            waitForValue(focusedAddress, containing: expectedAddressFragment, timeout: 5),
            "Expected focused address field to show \(browserURL), got \(accessibilityValue(focusedAddress))"
        )
    }

    func testIrisWebAddressFieldSelectsURLAndStaysFocusedOnSecondTap() throws {
        let browserURL = try optionalEnvironment("IRIS_DRIVE_UI_TEST_BROWSER_URL")
            ?? localBrowserFooterFixtureURL()
        let app = launchApp(environment: [
            "IRIS_DRIVE_DEBUG_ACTION": "open-browser",
            "IRIS_DRIVE_DEBUG_BROWSER_URL": browserURL,
        ])

        let addressBar = app.descendants(matching: .any)["irisWebAddressField"]
        XCTAssertTrue(addressBar.waitForExistence(timeout: 15), app.debugDescription)
        waitForIrisBrowserToFinishLoading(in: app)

        XCTAssertTrue(app.buttons["irisWebAddressField"].waitForExistence(timeout: 5), app.debugDescription)
        app.buttons["irisWebAddressField"].tap()

        let focusedAddress = app.textFields["irisWebAddressField"]
        XCTAssertTrue(focusedAddress.waitForExistence(timeout: 5), app.debugDescription)
        focusedAddress.typeText("x")
        XCTAssertEqual(
            accessibilityValue(focusedAddress),
            "x",
            "Initial address focus should select the whole URL so typing replaces it"
        )

        focusedAddress.tap()
        RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        XCTAssertTrue(
            focusedAddress.exists,
            "Tapping the focused address field again should keep it editable"
        )
    }

    func testIrisWebBackButtonDoesNotLeakTapToPage() throws {
        let browserURL = try optionalEnvironment("IRIS_DRIVE_UI_TEST_BACK_HIT_URL")
            ?? localBackHitFixtureURL()
        let app = launchApp(environment: [
            "IRIS_DRIVE_DEBUG_ACTION": "open-browser",
            "IRIS_DRIVE_DEBUG_BROWSER_URL": browserURL,
        ])

        let address = app.descendants(matching: .any)["irisWebAddressField"]
        XCTAssertTrue(address.waitForExistence(timeout: 15), app.debugDescription)
        waitForIrisBrowserToFinishLoading(in: app)

        let backButton = app.buttons["irisWebBackButton"]
        XCTAssertTrue(backButton.waitForExistence(timeout: 5), app.debugDescription)

        app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            .withOffset(CGVector(dx: backButton.frame.midX, dy: backButton.frame.midY))
            .tap()

        address.tap()
        let focusedAddress = app.textFields["irisWebAddressField"]
        XCTAssertTrue(focusedAddress.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertFalse(
            waitForValue(focusedAddress, containing: "fell-through", timeout: 2),
            "Back button tap leaked to page content; address became \(accessibilityValue(focusedAddress))"
        )
    }

    private func assertOpenIrisAppsLoads(
        in app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let openIrisApps = app.buttons["Open Iris Apps"].firstMatch
        makeHittable(openIrisApps, in: app)
        openIrisApps.tap()

        let address = app.descendants(matching: .any)["irisWebAddressField"]
        XCTAssertTrue(address.waitForExistence(timeout: 35), app.debugDescription, file: file, line: line)
        waitForIrisBrowserToFinishLoading(in: app, file: file, line: line)

        XCTAssertFalse(app.staticTexts["irisWebError"].exists, file: file, line: line)
        address.tap()
        let focusedAddress = app.textFields["irisWebAddressField"]
        XCTAssertTrue(focusedAddress.waitForExistence(timeout: 5), app.debugDescription, file: file, line: line)
        XCTAssertTrue(accessibilityValue(focusedAddress).contains("iris.localhost"), file: file, line: line)
        app.buttons["irisWebCloseButton"].tap()
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
        XCTAssertTrue(app.navigationBars["Devices"].waitForExistence(timeout: 10))
        XCTAssertTrue(tabButton("Devices", in: app).isSelected)
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

    func testBackupTabShowsFilesystemDestinationControls() throws {
        let baseDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-ui-test-backup-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: baseDir, withIntermediateDirectories: true)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: baseDir)
        }
        let app = launchApp(environment: [
            "IRIS_DRIVE_UI_TEST_BASE_DIR": baseDir.path,
        ])
        ensureMyDriveReady(in: app)

        let backupTab = tabButton("Backup", in: app)
        XCTAssertTrue(backupTab.waitForExistence(timeout: 10))
        backupTab.tap()

        XCTAssertTrue(app.navigationBars["Backup"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["upload.iris.to"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.textFields["backupTargetInput"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["addBackupButton"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Add Custom Target"].exists)
        XCTAssertFalse(app.buttons["Add File Server"].exists)
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
        XCTAssertTrue(tabButton("Devices", in: app).waitForExistence(timeout: 10))
        tabButton("Devices", in: app).tap()
        XCTAssertTrue(app.staticTexts["iOS UI linked"].waitForExistence(timeout: 10))
        XCTAssertTrue(
            waitForLinkedOnlineDeviceRow(in: app, timeout: 10),
            "Expected a linked online device row. Static texts:\n\(staticTextLabels(in: app))"
        )
        XCTAssertFalse(app.staticTexts["Authorized"].exists)
    }

    func testAddLinkedDeviceFromDevices() throws {
        let linkedDevice = try requiredEnvironment("IRIS_DRIVE_UI_TEST_LINKED_DEVICE")
        let app = launchApp()

        XCTAssertTrue(tabButton("Devices", in: app).waitForExistence(timeout: 10))
        tabButton("Devices", in: app).tap()
        let addDeviceToggle = app.buttons["Add Device"]
        XCTAssertTrue(addDeviceToggle.waitForExistence(timeout: 10))
        addDeviceToggle.tap()

        let deviceField = app.textFields["manualDeviceId"]
        makeHittable(deviceField, in: app)
        XCTAssertEqual(deviceField.value as? String, linkedDevice)

        let nameField = app.textFields["manualDeviceName"]
        makeHittable(nameField, in: app)
        XCTAssertEqual(nameField.value as? String, "iOS UI linked")
        app.buttons["manualDeviceAdd"].tap()

        XCTAssertTrue(
            waitForStaticText("iOS UI linked", in: app, timeout: 15),
            "Expected linked device row. Static texts:\n\(staticTextLabels(in: app))"
        )
    }

    private func launchApp(environment overrides: [String: String] = [:]) -> XCUIApplication {
        let app = XCUIApplication()
        for (key, value) in ProcessInfo.processInfo.environment
            where key.hasPrefix("IRIS_DRIVE_UI_TEST_") || key.hasPrefix("IRIS_DRIVE_FIPS_") {
            app.launchEnvironment[key] = value
        }
        for (key, value) in overrides {
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
        timeout: TimeInterval,
        expectedItem: String? = nil
    ) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if files.state == .runningForeground {
                if let expectedItem {
                    if filesContains(expectedItem, in: files) {
                        return
                    }
                    let driveLocation = files.descendants(matching: .any)["Iris Drive"].firstMatch
                    if driveLocation.exists, driveLocation.isHittable {
                        driveLocation.tap()
                    }
                } else if files.descendants(matching: .any)["Iris Drive"].exists {
                    return
                }
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

    private func filesContains(_ expectedItem: String, in files: XCUIApplication) -> Bool {
        let elements = files.descendants(matching: .any)
        if elements[expectedItem].exists {
            return true
        }
        let url = URL(fileURLWithPath: expectedItem)
        let displayStem = url.deletingPathExtension().lastPathComponent
        return !displayStem.isEmpty && displayStem != expectedItem && elements[displayStem].exists
    }

    private func requiredEnvironment(_ name: String) throws -> String {
        let environment = ProcessInfo.processInfo.environment
        let value = environment[name] ?? decodedEnvironmentValue("\(name)_B64", environment: environment)
        if value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            throw XCTSkip("\(name) is required for this UI test")
        }
        return value
    }

    private func optionalEnvironment(_ name: String) -> String? {
        let environment = ProcessInfo.processInfo.environment
        let value = environment[name] ?? decodedEnvironmentValue("\(name)_B64", environment: environment)
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    private func localBackHitFixtureURL() throws -> String {
        let url = URL(string: "http://127.0.0.1:8765/back-hit-test.html")!
        guard httpURLResponds(url) else {
            throw XCTSkip("IRIS_DRIVE_UI_TEST_BACK_HIT_URL or local fixture server is required")
        }
        return url.absoluteString
    }

    private func localBrowserFooterFixtureURL() throws -> String {
        let url = URL(string: "http://127.0.0.1:8765/browser-footer.html")!
        guard httpURLResponds(url) else {
            throw XCTSkip("IRIS_DRIVE_UI_TEST_BROWSER_URL or local fixture server is required")
        }
        return url.absoluteString
    }

    private func httpURLResponds(_ url: URL) -> Bool {
        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"
        request.timeoutInterval = 1
        let semaphore = DispatchSemaphore(value: 0)
        var ok = false
        URLSession.shared.dataTask(with: request) { _, response, _ in
            if let http = response as? HTTPURLResponse {
                ok = http.statusCode == 200
            }
            semaphore.signal()
        }.resume()
        _ = semaphore.wait(timeout: .now() + 2)
        return ok
    }

    private func decodedEnvironmentValue(_ name: String, environment: [String: String]) -> String {
        guard let encoded = environment[name],
              let data = Data(base64Encoded: encoded),
              let value = String(data: data, encoding: .utf8)
        else {
            return ""
        }
        return value
    }

    private func makeHittable(_ element: XCUIElement, in app: XCUIApplication) {
        for _ in 0..<6 where !element.isHittable {
            app.swipeUp()
        }
        XCTAssertTrue(
            element.waitForExistence(timeout: 2),
            "Expected element to exist. App hierarchy:\n\(app.debugDescription)"
        )
        XCTAssertTrue(element.isHittable, "Expected element to be hittable: \(element.debugDescription)")
    }

    private func accessibilityValue(_ element: XCUIElement) -> String {
        let value = (element.value as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !value.isEmpty {
            return value
        }
        return element.label.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func waitForStaticText(
        _ label: String,
        in app: XCUIApplication,
        timeout: TimeInterval
    ) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if app.staticTexts[label].exists {
                return true
            }
            app.swipeUp()
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        }
        return app.staticTexts[label].exists
    }

    private func waitForValue(
        _ element: XCUIElement,
        containing expected: String,
        timeout: TimeInterval
    ) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if accessibilityValue(element).contains(expected) {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        return accessibilityValue(element).contains(expected)
    }

    private func waitForIrisBrowserToFinishLoading(
        in app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let deadline = Date().addingTimeInterval(8)
        while Date() < deadline {
            let error = app.staticTexts["irisWebError"]
            XCTAssertFalse(error.exists, "Iris Apps browser failed: \(accessibilityValue(error))", file: file, line: line)
            if !app.progressIndicators["irisWebLoading"].exists {
                break
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTAssertFalse(app.staticTexts["irisWebError"].exists, file: file, line: line)
    }

    private func saveScreenshot(named name: String, in directory: String) throws {
        let directoryURL = URL(fileURLWithPath: directory, isDirectory: true)
        try FileManager.default.createDirectory(at: directoryURL, withIntermediateDirectories: true)
        let screenshot = XCUIScreen.main.screenshot()
        try screenshot.pngRepresentation.write(to: directoryURL.appendingPathComponent("\(name).png"))
        XCTContext.runActivity(named: name) { activity in
            let attachment = XCTAttachment(screenshot: screenshot)
            attachment.name = name
            attachment.lifetime = .keepAlways
            activity.add(attachment)
        }
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
