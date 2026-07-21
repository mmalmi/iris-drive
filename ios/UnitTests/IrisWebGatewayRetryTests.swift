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
}
