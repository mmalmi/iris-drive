import FileProvider
import UniformTypeIdentifiers
import XCTest
@testable import IrisDriveIOS

/// The "Save to Iris Drive" sheet: nothing is saved until asked, the person
/// chooses the name and folder, and a failure stays on screen.
@MainActor
final class ShareSaveModelTests: XCTestCase {
    private func item(_ name: String) -> PendingSharedItem {
        PendingSharedItem(
            name: name,
            contentType: .plainText,
            source: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        )
    }

    func testWithoutAProfileItSaysToSetUpFirst() {
        let model = ShareSaveModel()
        model.loaded([item("Notes.txt")], folders: nil)
        XCTAssertEqual(model.phase, .notSetUp)
        XCTAssertFalse(model.canSave)
    }

    func testNothingUsableIsSaidPlainly() {
        let model = ShareSaveModel()
        model.loaded([], folders: [])
        XCTAssertEqual(model.phase, .failed("Nothing here can be saved to Iris Drive."))
    }

    func testShareDestinationsWaitForVerifiedFolders() async throws {
        let model = ShareSaveModel()
        let staged = item("Notes.txt")
        let parent = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let config = parent.appendingPathComponent("profile")
        defer { try? FileManager.default.removeItem(at: parent) }
        let core = IrisDriveNativeCore(dataDir: config.path, appVersion: "test")
        _ = core.dispatchJson(#"{"type":"create_profile","app_key_label":"Share test"}"#)
        _ = IrisDriveNativeProvider.mkdir(dataDir: config.path, path: "Trip")
        struct Listing: Decodable { let save_folders: [String]?; let error: String? }
        @Sendable func listing() throws -> Listing {
            let text = IrisDriveNativeProvider.list(dataDir: config.path)
            return try JSONDecoder().decode(Listing.self, from: Data(text.utf8))
        }
        let expectedFolders = try XCTUnwrap(try listing().save_folders)
        XCTAssertTrue(expectedFolders.contains("Trip"))
        let blocks = config.appendingPathComponent("blocks")
        let held = parent.appendingPathComponent("in-transit")
        try FileManager.default.moveItem(at: blocks, to: held)
        let reads = Recorder()
        await model.prepareDestinations(for: [staged], delays: [.zero]) {
            reads.add("read")
            let response = try listing()
            if let error = response.error {
                // Deliver the withheld blocks after this real provider read has failed.
                if FileManager.default.fileExists(atPath: blocks.path) {
                    try FileManager.default.moveItem(at: blocks, to: parent.appendingPathComponent("empty-store"))
                }
                try FileManager.default.moveItem(at: held, to: blocks)
                throw NSError(domain: NSFileProviderErrorDomain, code: NSFileProviderError.serverUnreachable.rawValue,
                              userInfo: [NSLocalizedDescriptionKey: error])
            }
            return response.save_folders
        }
        XCTAssertEqual(reads.values.count, 2)
        XCTAssertEqual(model.items, [staged])
        XCTAssertEqual(model.folders, expectedFolders)
        XCTAssertEqual(model.phase, .ready)
    }

    func testShareDestinationsPreserveStagingWhenSyncIsStillPending() async {
        let model = ShareSaveModel()
        let staged = item("Notes.txt")
        let chunk = String(repeating: "b", count: 64)
        let pending = NSError(domain: NSFileProviderErrorDomain, code: NSFileProviderError.serverUnreachable.rawValue,
                              userInfo: [NSLocalizedDescriptionKey: "building provider view: tree: Missing chunk: " + chunk])
        await model.prepareDestinations(for: [staged], delays: []) { throw pending }
        XCTAssertEqual(model.phase, .waitingForFolders)
        XCTAssertEqual(model.items, [staged])
        XCTAssertFalse(model.canSave)
        await model.prepareDestinations(for: model.items, delays: []) { ["Recovered"] }
        XCTAssertTrue(model.canSave)
        XCTAssertEqual(model.folders, ["Recovered"])
    }

    func testShareDestinationsReturnOtherErrorsImmediately() async {
        let model = ShareSaveModel()
        let reads = Recorder()
        await model.prepareDestinations(for: [item("Notes.txt")], delays: [.zero]) {
            reads.add("read")
            throw CocoaError(.fileReadNoPermission)
        }
        XCTAssertEqual(reads.values.count, 1)
        XCTAssertEqual(model.phase, .failed(CocoaError(.fileReadNoPermission).localizedDescription))
    }

    func testANameIsNeededBeforeSaving() {
        let model = ShareSaveModel()
        model.loaded([item("Notes.txt")], folders: ["Trip"])
        XCTAssertEqual(model.phase, .ready)
        XCTAssertTrue(model.canSave)
        model.items[0].name = "  "
        XCTAssertFalse(model.canSave)
    }

    func testSavesTheRenamedItemIntoTheChosenFolderAndSaysWhere() async {
        let model = ShareSaveModel()
        model.loaded([item("Notes.txt")], folders: ["Projects", "Projects/Trip"])
        model.items[0].name = "Packing.txt"
        model.folder = "Projects/Trip"
        let saved = Recorder()
        let done = expectation(description: "sheet closes")

        model.save(using: { item, folder in saved.add("\(folder)/\(item.name)") }, done: { done.fulfill() })

        XCTAssertEqual(model.phase, .saving)
        await fulfillment(of: [done], timeout: 5)
        XCTAssertEqual(saved.values, ["Projects/Trip/Packing.txt"])
        XCTAssertEqual(model.phase, .saved("Saved to Trip"))
    }

    func testSeveralItemsGoToMyDriveByDefault() async {
        let model = ShareSaveModel()
        model.loaded([item("a.jpg"), item("b.jpg")], folders: [])
        let saved = Recorder()
        let done = expectation(description: "sheet closes")

        model.save(using: { item, folder in saved.add("\(folder)/\(item.name)") }, done: { done.fulfill() })

        await fulfillment(of: [done], timeout: 5)
        XCTAssertEqual(saved.values, ["/a.jpg", "/b.jpg"])
        XCTAssertEqual(model.phase, .saved("Saved 2 items to My Drive"))
    }

    func testAFailureStaysOnScreenAndTheSheetDoesNotClose() async {
        let model = ShareSaveModel()
        model.loaded([item("Notes.txt")], folders: [])
        let closed = expectation(description: "sheet closes")
        closed.isInverted = true

        model.save(using: { _, _ in throw CocoaError(.fileWriteOutOfSpace) }, done: { closed.fulfill() })

        await fulfillment(of: [closed], timeout: 1.5)
        guard case .failed = model.phase else {
            return XCTFail("expected a failure, got \(model.phase)")
        }
    }

    func testOfflineSaveExplainsWhereTheMovieIsAndKeepsTheSheetOpen() async {
        let model = ShareSaveModel()
        model.loaded([item("Trip.mov")], folders: [])
        let pending = "Saved on this iPhone. Waiting to sync."
        let closed = expectation(description: "dismissal")
        closed.isInverted = true
        model.save(using: { _, _ in }, sync: { pending }, done: { closed.fulfill() })
        await fulfillment(of: [closed], timeout: 1.1)
        XCTAssertEqual(model.phase, .saved(pending))
    }
}

private final class Recorder: @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [String] = []

    func add(_ value: String) {
        lock.lock()
        recorded.append(value)
        lock.unlock()
    }

    var values: [String] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }
}
