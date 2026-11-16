import UniformTypeIdentifiers
import XCTest

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
