import Photos
import XCTest
@testable import IrisDriveIOS

final class PhotoBackupTests: XCTestCase {
    func testOriginalsIncludeLiveMotionAndAlternatePhoto() {
        XCTAssertTrue(PhotoBackupLibrary.isOriginal(.photo))
        XCTAssertTrue(PhotoBackupLibrary.isOriginal(.video))
        XCTAssertTrue(PhotoBackupLibrary.isOriginal(.pairedVideo))
        XCTAssertTrue(PhotoBackupLibrary.isOriginal(.alternatePhoto))
        XCTAssertFalse(PhotoBackupLibrary.isOriginal(.adjustmentData))
        XCTAssertFalse(PhotoBackupLibrary.isOriginal(.fullSizePhoto))
        XCTAssertFalse(PhotoBackupLibrary.isOriginal(.photoProxy))
    }

    func testResourceIdentitySurvivesRelaunchAndDistinguishesSameNames() {
        func key(_ installation: String = "a", _ asset: String = "asset", _ type: Int = 1, _ occurrence: Int = 0) -> String {
            PhotoBackupLibrary.resourceID(installation: installation, asset: asset, type: type, filename: "IMG_0001.jpg", occurrence: occurrence)
        }
        XCTAssertEqual(key(), key())
        XCTAssertEqual(Set([key(), key("b"), key("a", "other"), key("a", "asset", 9), key("a", "asset", 1, 1)]).count, 5)
    }

    func testReceiptsSurviveRestartAndStayInTheirDrive() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("receipts.sqlite")
        do {
            let ledger = try PhotoBackupReceipts(url: url, profile: "alice")
            XCTAssertFalse(try ledger.contains("original"))
            try ledger.remember("original")
            try ledger.remember("original")
        }
        XCTAssertTrue(try PhotoBackupReceipts(url: url, profile: "alice").contains("original"))
        XCTAssertFalse(try PhotoBackupReceipts(url: url, profile: "bob").contains("original"))
    }
}

#if targetEnvironment(simulator)
import UIKit

extension PhotoBackupTests {
    /// The reserved simulator keeps one reusable test image. Only that image enters the test profile.
    func testPhotoKitOriginalRoundTripsAndSurvivesAChangeInSelection() async throws {
        try XCTSkipUnless(PHPhotoLibrary.authorizationStatus(for: .readWrite) == .authorized,
                          "Grant this test app Photos access on the reserved simulator.")
        let image = UIGraphicsImageRenderer(size: CGSize(width: 4, height: 4)).image { ctx in
            UIColor.blue.setFill()
            ctx.fill(CGRect(x: 0, y: 0, width: 4, height: 4))
        }
        let bytes = try XCTUnwrap(image.jpegData(compressionQuality: 0.9))
        let assets = try await testImage(bytes)
        XCTAssertEqual(assets.count, 1)
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        let config = dir.appendingPathComponent("config").path
        let app = IrisDriveNativeCore(dataDir: config, appVersion: "test")
        let json = app.dispatchJson(#"{"type":"create_profile","app_key_label":"Photo test"}"#)
        let state = try JSONDecoder().decode(NativeAppState.self, from: Data(json.utf8))
        let profile = try XCTUnwrap(state.ui.profile?.profileId)
        let scope = PhotoBackupScope(dataDir: config, profile: profile, installation: "test")
        func run(_ selected: PHFetchResult<PHAsset>) async throws -> String {
            try await PhotoBackupLibrary.backUp(selected, scope: scope, support: dir,
                staging: dir.appendingPathComponent("stage"), limit: nil, networkAllowed: false, networkForPublish: { false }) { _ in }
        }
        let nothingSelected = PHAsset.fetchAssets(withLocalIdentifiers: [], options: nil)
        let emptyStatus = try await run(nothingSelected)
        XCTAssertEqual(emptyStatus, "Up to date")
        for _ in 0..<2 {
            let status = try await run(assets)
            XCTAssertEqual(status, "Saved in My Drive · waiting to sync")
        }
        let removedStatus = try await run(nothingSelected)
        XCTAssertEqual(removedStatus, "Saved in My Drive · waiting to sync")
        let listing = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(IrisDriveNativeProvider.list(dataDir: config).utf8)) as? [String: Any])
        let files = try XCTUnwrap(listing["entries"] as? [[String: Any]]).filter { $0["kind"] as? String == "file" }
        XCTAssertEqual(files.count, 1)
        let path = try XCTUnwrap(files.first?["path"] as? String)
        XCTAssertTrue(path.hasPrefix("Photos/"))
        let output = dir.appendingPathComponent("original.jpg")
        _ = IrisDriveNativeProvider.read(dataDir: config, path: path, outputPath: output.path)
        // Photos can add metadata when it ingests a JPEG. Compare against the original
        // Photos actually stores, and independently verify that it is our synthetic image.
        let asset = try XCTUnwrap(assets.firstObject)
        let resource = try XCTUnwrap(PHAssetResource.assetResources(for: asset).first {
            $0.type == .photo && $0.originalFilename == "Iris Drive photo backup test.jpg"
        })
        let original = dir.appendingPathComponent("photokit-original.jpg")
        try await PHAssetResourceManager.default().writeData(for: resource, toFile: original, options: nil)
        let originalBytes = try Data(contentsOf: original)
        XCTAssertEqual(try pixels(in: originalBytes), try pixels(in: bytes))
        XCTAssertEqual(try Data(contentsOf: output), originalBytes)
    }

    private func pixels(in data: Data) throws -> Data {
        let image = try XCTUnwrap(UIImage(data: data)?.cgImage)
        let context = try XCTUnwrap(CGContext(data: nil, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: image.width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return Data(bytes: try XCTUnwrap(context.data), count: image.width * image.height * 4)
    }

    private func testImage(_ bytes: Data) async throws -> PHFetchResult<PHAsset> {
        let key = "photoBackupSimulatorTestAsset"
        if let id = UserDefaults.standard.string(forKey: key) {
            let existing = PHAsset.fetchAssets(withLocalIdentifiers: [id], options: nil)
            if existing.count == 1 { return existing }
        }
        var id: String?
        try await PHPhotoLibrary.shared().performChanges {
            let create = PHAssetCreationRequest.forAsset()
            let options = PHAssetResourceCreationOptions()
            options.originalFilename = "Iris Drive photo backup test.jpg"
            create.addResource(with: .photo, data: bytes, options: options)
            id = create.placeholderForCreatedAsset?.localIdentifier
        }
        let identifier = try XCTUnwrap(id)
        UserDefaults.standard.set(identifier, forKey: key)
        return PHAsset.fetchAssets(withLocalIdentifiers: [identifier], options: nil)
    }
}
#endif
