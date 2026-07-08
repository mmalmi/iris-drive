import XCTest

extension IrisDriveIOSUITests {
    func testWelcomeRoutesWithoutSetupTitle() throws {
        let app = launchApp(environment: try isolatedBaseEnvironment())
        XCTAssertFalse(app.navigationBars["Setup"].exists)
        XCTAssertTrue(app.images["brandLogo"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Iris Drive"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["welcomeCreateProfile"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["welcomeCreateProfile"].label.contains("Create profile"))
        XCTAssertTrue(app.buttons["welcomeSignIn"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["welcomeSignIn"].label.contains("Sign in"))

        app.buttons["welcomeCreateProfile"].tap()
        XCTAssertTrue(app.navigationBars["Create profile"].waitForExistence(timeout: 5))

        app.terminate()
        app.launch()
        app.buttons["welcomeSignIn"].tap()
        XCTAssertTrue(app.navigationBars["Restore"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["openRecoveryPhrase"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["openRecoveryPhrase"].label.contains("Restore from recovery phrase"))
        XCTAssertTrue(app.buttons["openSecretKey"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["openSecretKey"].label.contains("Restore from secret key"))
        XCTAssertTrue(app.buttons["openLinkDevice"].label.contains("Link device"))
        app.buttons["openLinkDevice"].tap()
        let startJoinRequest = app.buttons["startJoinRequest"]
        let awaitingApproval = app.descendants(matching: .any)["awaitingApprovalView"]
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline, !startJoinRequest.exists, !awaitingApproval.exists {
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        }
        XCTAssertTrue(startJoinRequest.exists || awaitingApproval.exists)
        XCTAssertFalse(app.textFields["linkTargetInput"].exists)
        XCTAssertFalse(app.staticTexts["Device invite link"].exists)
    }

    func testLinkThisDeviceFromWelcome() throws {
        let app = launchApp(environment: try isolatedBaseEnvironment())

        app.buttons["welcomeSignIn"].tap()
        app.buttons["openLinkDevice"].tap()

        let awaitingApproval = app.descendants(matching: .any)["awaitingApprovalView"]
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if awaitingApproval.exists {
                XCTAssertFalse(app.textFields["linkTargetInput"].exists)
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTFail(app.debugDescription)
    }

    func testAwaitingApprovalViewVisible() throws {
        let app = launchApp()

        let awaitingApproval = app.descendants(matching: .any)["awaitingApprovalView"]
        XCTAssertTrue(awaitingApproval.waitForExistence(timeout: 15), app.debugDescription)
        XCTAssertFalse(app.textFields["linkTargetInput"].exists)
    }

    func testLinkDeviceDoesNotRenderInviteInput() throws {
        let app = launchApp(environment: try isolatedBaseEnvironment())

        app.buttons["welcomeSignIn"].tap()
        app.buttons["openLinkDevice"].tap()

        let startJoinRequest = app.buttons["startJoinRequest"]
        let awaitingApproval = app.descendants(matching: .any)["awaitingApprovalView"]
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline, !startJoinRequest.exists, !awaitingApproval.exists {
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        }
        XCTAssertTrue(startJoinRequest.exists || awaitingApproval.exists)
        XCTAssertFalse(app.textFields["linkTargetInput"].exists)
        XCTAssertFalse(app.staticTexts["Device invite link"].exists)
    }

    func testCreateProfileFromWelcome() throws {
        let app = launchApp()

        app.buttons["welcomeCreateProfile"].tap()
        app.buttons["createProfileSubmit"].tap()

        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 15))
    }

    func testCreateProfileWithUsernameCanSkipProfilePhoto() throws {
        let app = launchApp()

        app.buttons["welcomeCreateProfile"].tap()
        let username = app.textFields["createUsername"]
        XCTAssertTrue(username.waitForExistence(timeout: 5))
        username.tap()
        username.typeText("Ada")
        app.buttons["createProfileSubmit"].tap()

        XCTAssertTrue(app.navigationBars["Profile photo"].waitForExistence(timeout: 5))
        let photoSubmit = app.buttons["createPhotoSubmit"]
        XCTAssertTrue(photoSubmit.waitForExistence(timeout: 5), app.debugDescription)
        photoSubmit.tap()
        if !tabButton("My Drive", in: app).waitForExistence(timeout: 8), photoSubmit.exists {
            photoSubmit.tap()
        }

        XCTAssertTrue(tabButton("My Drive", in: app).waitForExistence(timeout: 20), app.debugDescription)
    }
}
