import XCTest
@testable import IrisDriveIOS

final class IrisWebGatewayRetryTests: XCTestCase {
    func testResolverMissIsTransientForLocalGatewayRoutes() throws {
        let url = try XCTUnwrap(
            URL(string: "http://sites.npub1example.iris.localhost:17321/")
        )

        XCTAssertTrue(
            irisWebIsTransientGatewayNotFound(
                "Resolution failed through configured event provider and peers",
                url: url
            )
        )
        XCTAssertTrue(
            irisWebIsTransientGatewayNotFound(
                "Root not found through configured event provider",
                url: url
            )
        )
    }

    func testResolverMissIsNotTransientForExternalRoutes() throws {
        let url = try XCTUnwrap(URL(string: "https://apps.iris.to/"))

        XCTAssertFalse(
            irisWebIsTransientGatewayNotFound(
                "Resolution failed through configured event provider and peers",
                url: url
            )
        )
    }

    func testMaterializedLauncherDoesNotRequireNavigationDelegateCompletion() {
        XCTAssertTrue(
            irisDebugWebViewIsMaterialized(
                title: "Iris: The Freedom Toolkit",
                bodyText: "Iris: The Freedom Toolkit\nDrive\nChat\nContacts",
                readyState: "complete",
                htmlLength: 15_353,
                htmlPrefix: "<html><body><div id=\"app\"><main>Drive</main></div></body></html>",
                screenshotPath: "/tmp/iris-apps.png",
                screenshotError: ""
            )
        )
    }

    func testMaterializedLauncherFailsClosedOnIncompleteOrWrongContent() {
        let valid = (
            title: "Iris: The Freedom Toolkit",
            body: "Iris: The Freedom Toolkit\nDrive\nChat\nContacts",
            html: "<html><body><div id=\"app\"><main>Drive</main></div></body></html>"
        )
        XCTAssertFalse(
            irisDebugWebViewIsMaterialized(
                title: valid.title,
                bodyText: valid.body,
                readyState: "loading",
                htmlLength: 15_353,
                htmlPrefix: valid.html,
                screenshotPath: "/tmp/iris-apps.png",
                screenshotError: ""
            )
        )
        XCTAssertFalse(
            irisDebugWebViewIsMaterialized(
                title: "Not Found",
                bodyText: "Not found",
                readyState: "complete",
                htmlLength: 15_353,
                htmlPrefix: valid.html,
                screenshotPath: "/tmp/iris-apps.png",
                screenshotError: ""
            )
        )
        XCTAssertFalse(
            irisDebugWebViewIsMaterialized(
                title: valid.title,
                bodyText: valid.body,
                readyState: "complete",
                htmlLength: 15_353,
                htmlPrefix: valid.html,
                screenshotPath: "",
                screenshotError: "Snapshot failed"
            )
        )
    }

    @MainActor
    func testProbeDeadlineCompletesWhenCaptureCallbackNeverArrives() {
        let completion = IrisDebugProbeCompletion()
        var results: [String] = []
        var teardownCount = 0
        completion.start(timeoutMilliseconds: 30_000) {
            teardownCount += 1
            results.append("timeout")
        }
        let pendingCaptureCallback = { completion.complete { results.append("success") } }
        completion.deadlineExpired()
        XCTAssertEqual(results, ["timeout"])
        XCTAssertEqual(teardownCount, 1)
        XCTAssertTrue(completion.completed)
        _ = pendingCaptureCallback
        completion.complete {}
    }

    @MainActor
    func testProbeDeadlineIgnoresLateCaptureAndRepeatedExpiry() {
        let completion = IrisDebugProbeCompletion()
        var results: [String] = []
        completion.start(timeoutMilliseconds: 30_000) { results.append("timeout") }
        completion.deadlineExpired()
        completion.deadlineExpired()
        XCTAssertFalse(completion.complete { results.append("late success") })
        XCTAssertEqual(results, ["timeout"])
    }

    @MainActor
    func testProbeSuccessfulCaptureWinsBeforeDeadline() {
        let completion = IrisDebugProbeCompletion()
        var results: [String] = []
        completion.start(timeoutMilliseconds: 30_000) { results.append("timeout") }
        XCTAssertTrue(completion.complete { results.append("validated complete result") })
        completion.deadlineExpired()
        XCTAssertFalse(completion.complete { results.append("duplicate") })
        XCTAssertEqual(results, ["validated complete result"])
    }


    @MainActor
    func testProbeConfiguredDeadlineDoesNotWaitForCapture() async {
        let completion = IrisDebugProbeCompletion()
        let expired = expectation(description: "overall probe deadline")
        completion.start(timeoutMilliseconds: 1) { expired.fulfill() }
        await fulfillment(of: [expired], timeout: 1)
        completion.complete {}
    }

}
