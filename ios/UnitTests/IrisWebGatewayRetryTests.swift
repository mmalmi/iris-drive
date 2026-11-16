import XCTest
@testable import IrisDriveIOS

final class IrisWebGatewayRetryTests: XCTestCase {
    func testOnlyTheDisposableBrowserProbeSkipsTheInstalledFilesDomain() {
        #if DEBUG
        let temporary = ["IRIS_DRIVE_UI_TEST_BASE_DIR": "__TMP__/browser",
                         "IRIS_DRIVE_DEBUG_ACTION": "probe-iris-apps"]
        XCTAssertTrue(IrisDriveSharedContainer.isIsolatedBrowserProbe(temporary))
        XCTAssertFalse(IrisDriveSharedContainer.isIsolatedBrowserProbe([:]))
        XCTAssertFalse(IrisDriveSharedContainer.isIsolatedBrowserProbe(["IRIS_DRIVE_DEBUG_ACTION": "probe-iris-apps"]))
        XCTAssertFalse(IrisDriveSharedContainer.isIsolatedBrowserProbe(["IRIS_DRIVE_UI_TEST_BASE_DIR": "__TMP__/files-test"]))
        #else
        XCTAssertFalse(IrisDriveSharedContainer.isIsolatedBrowserProbe(["IRIS_DRIVE_UI_TEST_BASE_DIR": "__TMP__",
                                                                       "IRIS_DRIVE_DEBUG_ACTION": "probe-iris-apps"]))
        #endif
    }

    func testSuspendingClosesGatewayAndProfileRefreshCannotReopenIt() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: dir) }
        let core = IrisDriveNativeCore(dataDir: dir.path, appVersion: "test")
        let statusURL = dir.appendingPathComponent("native-browser-gateway-status.json")
        func status() -> IrisNativeBrowserGatewayStatus? {
            guard let data = try? Data(contentsOf: statusURL) else { return nil }
            return try? JSONDecoder().decode(IrisNativeBrowserGatewayStatus.self, from: data)
        }
        func waitForRunningGateway() {
            let running = XCTNSPredicateExpectation(
                predicate: NSPredicate { _, _ in status()?.running == true },
                object: nil
            )
            XCTAssertEqual(XCTWaiter.wait(for: [running], timeout: 15), .completed)
        }

        _ = core.dispatchJson(#"{"type":"replace_relays","urls":["ws://127.0.0.1:1"]}"#)
        _ = core.dispatchJson(#"{"type":"replace_blossom_servers","urls":["http://127.0.0.1:1"]}"#)
        _ = core.dispatchJson(#"{"type":"create_profile","app_key_label":"Gateway test"}"#)
        core.setGatewayForeground(true)
        waitForRunningGateway()
        core.setGatewayForeground(false)
        XCTAssertEqual(status()?.state, "stopped")
        _ = core.dispatchJson(#"{"type":"refresh_profile"}"#)
        XCTAssertEqual(status()?.state, "stopped")
        core.setGatewayForeground(true)
        waitForRunningGateway()
        core.setGatewayForeground(false)
    }

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
    func testProbeDeadlinePreservesPartialEvidenceAndRejectsLateProgress() {
        let completion = IrisDebugProbeCompletion()
        var timeoutFields: [String: Any] = [:]
        completion.record(["phase": "gateway", "opened": false])
        completion.start(timeoutMilliseconds: 30_000) {
            timeoutFields = completion.fields
        }
        completion.record(["phase": "webview.capture", "opened": true])
        completion.record(["webview_title": "Iris: The Freedom Toolkit"])
        completion.deadlineExpired()
        completion.record(["phase": "finished", "webview_loaded": true])

        XCTAssertEqual(timeoutFields["phase"] as? String, "webview.capture")
        XCTAssertEqual(timeoutFields["opened"] as? Bool, true)
        XCTAssertEqual(timeoutFields["webview_title"] as? String, "Iris: The Freedom Toolkit")
        XCTAssertNil(completion.fields["webview_loaded"])
        XCTAssertEqual(completion.fields["phase"] as? String, "webview.capture")
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
