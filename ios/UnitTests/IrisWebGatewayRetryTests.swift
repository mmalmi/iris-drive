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
}
