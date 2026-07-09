import XCTest

extension IrisDriveIOSUITests {
    func testWelcomeRoutesWithoutSetupTitle() throws {
        let app = launchApp(environment: try isolatedBaseEnvironment())
        XCTAssertFalse(app.navigationBars["Setup"].exists)
        XCTAssertTrue(app.descendants(matching: .any)["brandLogo"].waitForExistence(timeout: 5))
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
        XCTAssertTrue(app.buttons["openRecoveryPhrase"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["openRecoveryPhrase"].label.contains("Restore from recovery phrase"))
        XCTAssertTrue(app.buttons["openSecretKey"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["openSecretKey"].label.contains("Restore from secret key"))
        let copyRequestLink = app.buttons["copyRequestLink"]
        let awaitingApproval = app.descendants(matching: .any)["awaitingApprovalView"]
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline, !copyRequestLink.exists, !awaitingApproval.exists {
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        }
        XCTAssertTrue(copyRequestLink.exists || awaitingApproval.exists)
    }

    func testLinkThisDeviceFromWelcome() throws {
        let app = launchApp(environment: try isolatedBaseEnvironment())

        app.buttons["welcomeSignIn"].tap()

        let copyRequestLink = app.buttons["copyRequestLink"]
        let awaitingApproval = app.descendants(matching: .any)["awaitingApprovalView"]
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if awaitingApproval.exists || copyRequestLink.exists {
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
    }

    func testSignInStartsJoinRequest() throws {
        let app = launchApp(environment: try isolatedBaseEnvironment())

        app.buttons["welcomeSignIn"].tap()

        let copyRequestLink = app.buttons["copyRequestLink"]
        let awaitingApproval = app.descendants(matching: .any)["awaitingApprovalView"]
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline, !copyRequestLink.exists, !awaitingApproval.exists {
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        }
        XCTAssertTrue(copyRequestLink.exists || awaitingApproval.exists)
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
