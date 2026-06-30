import XCTest
import UIKit

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

    func testLinkDeviceShowsInvalidInviteReason() throws {
        let baseDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-ui-test-invalid-link-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: baseDir, withIntermediateDirectories: true)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: baseDir)
        }
        let app = launchApp(environment: [
            "IRIS_DRIVE_UI_TEST_BASE_DIR": baseDir.path,
            "IRIS_DRIVE_UI_TEST_OWNER_INVITE_B64": "aHR0cHM6Ly9kcml2ZS5pcmlzLnRvL2ludml0ZS9kZW1v",
        ])

        app.buttons["welcomeSignIn"].tap()
        app.buttons["openLinkDevice"].tap()

        let error = app.staticTexts["linkDeviceErrorMessage"]
        XCTAssertTrue(error.waitForExistence(timeout: 5))
        XCTAssertTrue(accessibilityValue(error).contains("full device invite"))
        XCTAssertFalse(app.descendants(matching: .any)["awaitingApprovalView"].exists)
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
        assertFilesOpen(in: app, files: files, timeout: 45, expectedItem: seededFile)
        #if targetEnvironment(simulator)
        return
        #else
        assertNoFilesProviderTrouble(in: files)
        #endif
    }

    func testShareSheetImportsFileFromExternalSender() throws {
        let sharedFile = optionalEnvironment("IRIS_DRIVE_UI_TEST_SHARE_SHEET_FILE")
            ?? "Iris Drive Share Sheet Smoke.txt"
        let sharedContents = optionalEnvironment("IRIS_DRIVE_UI_TEST_SHARE_SHEET_CONTENT")
            ?? "shared from iOS share sheet\n"
        let app = launchApp(environment: [
            "IRIS_DRIVE_DEBUG_ACTION": "reset-local-state",
        ])
        ensureMyDriveReady(in: app)

        let sender = XCUIApplication(
            bundleIdentifier: optionalEnvironment("IRIS_DRIVE_UI_TEST_SHARE_SOURCE_BUNDLE_ID")
                ?? "fi.siriusbusiness.drive.ShareSource"
        )
        sender.launchEnvironment["IRIS_DRIVE_SHARE_SOURCE_FILENAME"] = sharedFile
        sender.launchEnvironment["IRIS_DRIVE_SHARE_SOURCE_CONTENT"] = sharedContents
        sender.launch()
        addTeardownBlock {
            sender.terminate()
        }

        let shareButton = sender.buttons["shareFileToIrisDriveButton"]
        XCTAssertTrue(
            shareButton.waitForExistence(timeout: 10),
            "Share source app did not launch. Hierarchy:\n\(sender.debugDescription)"
        )
        shareButton.tap()

        tapShareExtensionAction(sourceApp: sender, timeout: 20)
        waitForShareSheetToDismiss(sourceApp: sender, timeout: 12)

        app.terminate()
        let refreshed = launchApp()
        ensureMyDriveReady(in: refreshed)
        let row = refreshed.descendants(matching: .any)["filesSummaryRow"]
        XCTAssertTrue(row.waitForExistence(timeout: 10), refreshed.debugDescription)
        XCTAssertTrue(
            waitForValue(row, containing: "1", timeout: 25),
            "Expected share extension import to appear in My Drive. Row: \(row.debugDescription)"
        )
        assertSharedFileVisibleInFiles(sharedFile, in: refreshed)
    }

    func testOpenIrisAppsLoadsBrowserWithoutConnectionError() throws {
        let app = launchApp()
        ensureMyDriveReady(in: app)

        assertOpenIrisAppsLoads(in: app)
    }

    func testOpenIrisAppsLoadsBrowserWhenSyncPaused() throws {
        let app = launchApp()
        ensureMyDriveReady(in: app)

        pauseSyncIfNeeded(in: app)
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

    func testIrisWebAddressFocusKeepsPageVisibleAboveKeyboard() throws {
        let browserURL = try optionalEnvironment("IRIS_DRIVE_UI_TEST_BROWSER_URL")
            ?? localKeyboardViewportFixtureURL()
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
        RunLoop.current.run(until: Date().addingTimeInterval(0.8))

        let screenshot = XCUIScreen.main.screenshot()
        if let screenshotDir = optionalEnvironment("IRIS_DRIVE_UI_SCREENSHOT_DIR") {
            try saveScreenshot(named: "iris-web-address-focused-keyboard", in: screenshotDir)
        }
        assertNoLargeBlankVoidAbove(
            focusedAddress,
            in: app,
            screenshot: screenshot
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

    func testIrisWebLauncherExternalLinksOpenSystemBrowser() throws {
        let browserURL = try optionalEnvironment("IRIS_DRIVE_UI_TEST_EXTERNAL_LINKS_URL")
            ?? localExternalLinksFixtureURL()
        let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
        safari.terminate()
        let app = launchApp(environment: [
            "IRIS_DRIVE_DEBUG_ACTION": "open-browser",
            "IRIS_DRIVE_DEBUG_BROWSER_URL": browserURL,
        ])
        addTeardownBlock {
            safari.terminate()
            app.terminate()
        }

        let address = app.descendants(matching: .any)["irisWebAddressField"]
        XCTAssertTrue(address.waitForExistence(timeout: 15), app.debugDescription)
        waitForIrisBrowserToFinishLoading(in: app)

        let protonLink = app.links["Proton Mail"].firstMatch
        if protonLink.waitForExistence(timeout: 2), protonLink.isHittable {
            protonLink.tap()
        } else {
            let webView = app.webViews.firstMatch
            XCTAssertTrue(webView.waitForExistence(timeout: 5), app.debugDescription)
            webView.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.22)).tap()
        }

        XCTAssertTrue(
            safari.wait(for: .runningForeground, timeout: 10),
            "Expected Proton launcher link to open Safari instead of staying inside Iris Web. App hierarchy:\n\(app.debugDescription)"
        )
    }

    private func assertOpenIrisAppsLoads(
        in app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let openIrisApps = app.buttons["openIrisAppsButton"].firstMatch
        makeHittable(openIrisApps, in: app)
        openIrisApps.tap()

        let address = app.descendants(matching: .any)["irisWebAddressField"]
        XCTAssertTrue(address.waitForExistence(timeout: 35), app.debugDescription, file: file, line: line)
        waitForIrisBrowserToFinishLoading(in: app, file: file, line: line)

        XCTAssertFalse(app.staticTexts["irisWebError"].exists, file: file, line: line)
        assertIrisAppsLauncherContentLoaded(in: app, file: file, line: line)
        address.tap()
        let focusedAddress = app.textFields["irisWebAddressField"]
        XCTAssertTrue(focusedAddress.waitForExistence(timeout: 5), app.debugDescription, file: file, line: line)
        XCTAssertTrue(accessibilityValue(focusedAddress).contains("iris.localhost"), file: file, line: line)
        app.buttons["irisWebCloseButton"].tap()
    }

    private func pauseSyncIfNeeded(in app: XCUIApplication) {
        if app.staticTexts["Sync paused"].waitForExistence(timeout: 2) {
            return
        }
        let pauseSync = app.buttons["Pause sync"].firstMatch
        makeHittable(pauseSync, in: app)
        pauseSync.tap()
        XCTAssertTrue(
            app.staticTexts["Sync paused"].waitForExistence(timeout: 5)
                || app.buttons["Resume sync"].firstMatch.waitForExistence(timeout: 5),
            app.debugDescription
        )
    }

    private func assertIrisAppsLauncherContentLoaded(
        in app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let webView = app.webViews.firstMatch
        XCTAssertTrue(webView.waitForExistence(timeout: 5), app.debugDescription, file: file, line: line)
        for marker in ["Drive"] {
            let predicate = NSPredicate(format: "label CONTAINS[c] %@", marker)
            let inWebView = webView.descendants(matching: .any).matching(predicate).firstMatch
            let anywhere = app.descendants(matching: .any).matching(predicate).firstMatch
            XCTAssertTrue(
                inWebView.waitForExistence(timeout: 10) || anywhere.exists,
                "Expected Iris Apps launcher marker \(marker). App hierarchy:\n\(app.debugDescription)",
                file: file,
                line: line
            )
        }
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

    func testDevicesTabDoesNotWaitForInviteQrRendering() throws {
        let baseDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-ui-test-devices-qr-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: baseDir, withIntermediateDirectories: true)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: baseDir)
        }
        let app = launchApp(environment: [
            "IRIS_DRIVE_UI_TEST_BASE_DIR": baseDir.path,
            "IRIS_DRIVE_DEBUG_QR_DELAY_MS": "6000",
        ])
        ensureMyDriveReady(in: app)

        let devices = tabButton("Devices", in: app)
        XCTAssertTrue(devices.waitForExistence(timeout: 10))
        let started = Date()
        devices.tap()

        XCTAssertTrue(app.navigationBars["Devices"].waitForExistence(timeout: 3), app.debugDescription)
        XCTAssertLessThan(Date().timeIntervalSince(started), 4)
        XCTAssertFalse(
            app.staticTexts["No devices yet"].exists,
            "Fresh profile should at least show the current device. Static texts:\n\(staticTextLabels(in: app))"
        )
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

    func testSettingsExposeAppleCalendarContinuousSyncToggle() throws {
        let app = launchApp()
        ensureMyDriveReady(in: app)

        tabButton("Settings", in: app).tap()

        let toggle = app.descendants(matching: .any)["appleCalendarSyncToggle"]
        let status = app.staticTexts["appleCalendarSyncStatus"]
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline, !toggle.exists {
            app.swipeUp()
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTAssertTrue(toggle.exists, app.debugDescription)
        XCTAssertTrue(status.exists, app.debugDescription)
        XCTAssertFalse(app.buttons["Sync now"].exists)
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
            waitForStaticText(linkedDevice, in: app, timeout: 15)
                && waitForStaticText("Member | Linked | Offline", in: app, timeout: 5),
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
        let activateFilesAfter = Date().addingTimeInterval(8)
        let shouldDirectlyActivateFiles = expectedItem == nil
        var activatedFilesDirectly = false
        while Date() < deadline {
            if shouldDirectlyActivateFiles,
               !activatedFilesDirectly,
               Date() >= activateFilesAfter,
               files.state != .runningForeground {
                files.activate()
                activatedFilesDirectly = true
            }
            if files.state == .runningForeground {
                #if targetEnvironment(simulator)
                if expectedItem != nil {
                    return
                }
                #endif
                if let trouble = filesProviderTrouble(in: files) {
                    XCTFail("Files showed Iris Drive provider trouble while opening: \(trouble)")
                    return
                }
                if let expectedItem {
                    if filesContains(expectedItem, in: files) {
                        return
                    }
                    let driveLocation = files.descendants(matching: .any)["Iris Drive"].firstMatch
                    if driveLocation.exists, driveLocation.isHittable {
                        driveLocation.tap()
                    }
                    let browseBack = files.buttons["BackButton"].firstMatch
                    if browseBack.exists, browseBack.isHittable {
                        files.coordinate(withNormalizedOffset: CGVector(dx: 0.095, dy: 0.096)).tap()
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
        #if targetEnvironment(simulator)
        if expectedItem != nil {
            XCTExpectFailure(
                "iOS Simulator sometimes accepts the Open in Files URL without foregrounding " +
                    "DocumentsApp. The smoke already verified the seeded provider item in-app " +
                    "and treats app-side registration/open errors as hard failures."
            ) {
                XCTFail(
                    "Simulator Files did not expose the Iris Drive location. " +
                        "Simulator Files did not foreground from Open in Files before timeout."
                )
            }
            return
        }
        #endif
        XCTFail("Files did not show Iris Drive. Files hierarchy:\n\(files.debugDescription)")
    }

    private func assertSharedFileVisibleInFiles(_ sharedFile: String, in app: XCUIApplication) {
        let files = XCUIApplication(bundleIdentifier: "com.apple.DocumentsApp")
        files.terminate()
        let openInFiles = app.buttons["openInFilesButton"]
        makeHittable(openInFiles, in: app)
        openInFiles.tap()
        assertFilesOpen(in: app, files: files, timeout: 25, expectedItem: sharedFile)
        #if targetEnvironment(simulator)
        return
        #else
        assertNoFilesProviderTrouble(in: files)
        #endif
    }

    private func assertNoFilesProviderTrouble(
        in files: XCUIApplication,
        duration: TimeInterval = 4,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let deadline = Date().addingTimeInterval(duration)
        while Date() < deadline {
            if let trouble = filesProviderTrouble(in: files) {
                XCTFail("Files showed Iris Drive provider trouble after opening: \(trouble)", file: file, line: line)
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
    }

    private func filesProviderTrouble(in files: XCUIApplication) -> String? {
        let exactTrouble = [
            "iris drive is empty",
            "syncing with iris drive paused",
            "unable to sync",
            "upload error",
            "download error",
        ]
        if let text = firstStaticText(containingAny: exactTrouble, in: files) {
            return text
        }
        guard let irisDriveText = firstStaticText(containingAny: ["iris drive"], in: files) else {
            return nil
        }
        if let detail = firstStaticText(
            containingAny: ["paused", "error", "couldn't", "couldn’t", "could not"],
            in: files
        ) {
            return "\(irisDriveText)\n\(detail)"
        }
        return nil
    }

    private func firstStaticText(containingAny needles: [String], in app: XCUIApplication) -> String? {
        for needle in needles {
            let predicate = NSPredicate(format: "label CONTAINS[c] %@", needle)
            let match = app.staticTexts.matching(predicate).firstMatch
            if match.exists {
                let text = accessibilityValue(match)
                return text.isEmpty ? needle : text
            }
        }
        return nil
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

    private func localKeyboardViewportFixtureURL() throws -> String {
        let url = URL(string: "http://127.0.0.1:8765/keyboard-viewport.html")!
        guard httpURLResponds(url) else {
            throw XCTSkip("IRIS_DRIVE_UI_TEST_BROWSER_URL or local fixture server is required")
        }
        return url.absoluteString
    }

    private func localExternalLinksFixtureURL() throws -> String {
        let url = URL(string: "http://127.0.0.1:8765/external-links.html")!
        guard httpURLResponds(url) else {
            throw XCTSkip("IRIS_DRIVE_UI_TEST_EXTERNAL_LINKS_URL or local fixture server is required")
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

    private func tapShareExtensionAction(sourceApp: XCUIApplication, timeout: TimeInterval) {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let candidates = [
            sourceApp,
            springboard,
            XCUIApplication(bundleIdentifier: "com.apple.SharingViewService"),
        ]
        let labels = ["Save to Iris Drive", "Iris Drive"]
        let deadline = Date().addingTimeInterval(timeout)
        var tappedMore = false

        while Date() < deadline {
            for candidate in candidates {
                for label in labels {
                    let button = candidate.buttons[label].firstMatch
                    if button.exists {
                        makeShareSheetElementHittable(button, in: candidate)
                        button.tap()
                        return
                    }
                    let cell = candidate.cells[label].firstMatch
                    if cell.exists {
                        makeShareSheetElementHittable(cell, in: candidate)
                        cell.tap()
                        return
                    }
                    let text = candidate.staticTexts[label].firstMatch
                    if text.exists {
                        makeShareSheetElementHittable(text, in: candidate)
                        text.tap()
                        return
                    }
                }
                if !tappedMore {
                    let more = candidate.buttons["More"].firstMatch
                    if more.exists {
                        makeShareSheetElementHittable(more, in: candidate)
                        more.tap()
                        tappedMore = true
                        break
                    }
                }
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTFail(
            "Could not find Save to Iris Drive in the system share sheet.\n" +
                "Sender:\n\(sourceApp.debugDescription)\nSpringBoard:\n\(springboard.debugDescription)"
        )
    }

    private func makeShareSheetElementHittable(_ element: XCUIElement, in app: XCUIApplication) {
        for _ in 0..<4 where !element.isHittable {
            app.swipeUp()
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        }
        XCTAssertTrue(element.exists, "Expected share sheet element to exist: \(element.debugDescription)")
        XCTAssertTrue(element.isHittable, "Expected share sheet element to be hittable: \(element.debugDescription)")
    }

    private func waitForShareSheetToDismiss(sourceApp: XCUIApplication, timeout: TimeInterval) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if sourceApp.buttons["shareFileToIrisDriveButton"].isHittable {
                return
            }
            if sourceApp.staticTexts["Saved to Iris Drive"].exists {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
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

    private func assertNoLargeBlankVoidAbove(
        _ element: XCUIElement,
        in app: XCUIApplication,
        screenshot: XCUIScreenshot,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        guard let image = UIImage(data: screenshot.pngRepresentation),
              let cgImage = image.cgImage,
              let samples = RGBAImage(cgImage: cgImage)
        else {
            XCTFail("Could not read focused browser screenshot pixels", file: file, line: line)
            return
        }

        let scale = CGFloat(samples.width) / max(app.frame.width, 1)
        let fieldFrame = element.frame
        let sampleFrame = CGRect(
            x: max(app.frame.minX + 24, fieldFrame.minX - 72),
            y: max(app.frame.minY + 160, fieldFrame.minY - 260),
            width: min(app.frame.width - 48, fieldFrame.width + 144),
            height: 180
        )
        let pixelRect = CGRect(
            x: sampleFrame.minX * scale,
            y: sampleFrame.minY * scale,
            width: sampleFrame.width * scale,
            height: min(sampleFrame.height, max(fieldFrame.minY - sampleFrame.minY - 20, 1)) * scale
        ).integral

        var blankVoid = 0
        var total = 0
        let step = max(4, Int(scale * 6))
        let minX = max(0, Int(pixelRect.minX))
        let maxX = min(samples.width - 1, Int(pixelRect.maxX))
        let minY = max(0, Int(pixelRect.minY))
        let maxY = min(samples.height - 1, Int(pixelRect.maxY))

        for y in stride(from: minY, through: maxY, by: step) {
            for x in stride(from: minX, through: maxX, by: step) {
                total += 1
                if samples.isBlankVoid(x: x, y: y) {
                    blankVoid += 1
                }
            }
        }

        XCTAssertGreaterThan(total, 0, "Focused browser screenshot sample was empty", file: file, line: line)
        let ratio = total == 0 ? 1 : Double(blankVoid) / Double(total)
        XCTAssertLessThan(
            ratio,
            0.45,
            "Focused address left a blank page void above the keyboard (blank ratio \(ratio))",
            file: file,
            line: line
        )
    }

    private struct RGBAImage {
        let width: Int
        let height: Int
        private let bytes: [UInt8]

        init?(cgImage: CGImage) {
            width = cgImage.width
            height = cgImage.height
            var data = [UInt8](repeating: 0, count: width * height * 4)
            guard let context = CGContext(
                data: &data,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else {
                return nil
            }
            context.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))
            bytes = data
        }

        func isBlankVoid(x: Int, y: Int) -> Bool {
            let offset = ((y * width) + x) * 4
            guard offset + 3 < bytes.count else { return false }
            let red = bytes[offset]
            let green = bytes[offset + 1]
            let blue = bytes[offset + 2]
            let alpha = bytes[offset + 3]
            guard alpha > 180 else { return false }
            let nearBlack = red < 22 && green < 22 && blue < 22
            let nearWhite = red > 238 && green > 238 && blue > 238
            return nearBlack || nearWhite
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
