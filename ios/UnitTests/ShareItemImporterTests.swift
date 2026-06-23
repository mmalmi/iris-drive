import UniformTypeIdentifiers
import XCTest

final class ShareItemImporterTests: XCTestCase {
    private struct SavedFile {
        var displayName: String
        var contentType: UTType
        var contents: Data
    }

    private final class SaveRecorder {
        var files: [SavedFile] = []
    }

    func testPreferredTypeChoosesPlainTextBeforeGenericData() {
        let provider = NSItemProvider()
        provider.registerDataRepresentation(forTypeIdentifier: UTType.data.identifier, visibility: .all) { completion in
            completion(Data([0x01]), nil)
            return nil
        }
        provider.registerDataRepresentation(forTypeIdentifier: UTType.plainText.identifier, visibility: .all) { completion in
            completion(Data("hello".utf8), nil)
            return nil
        }

        let importer = makeImporter()

        XCTAssertEqual(importer.preferredTypeIdentifier(for: provider), UTType.plainText.identifier)
    }

    func testTextImportUsesSuggestedNameAndAddsExtension() throws {
        let recorder = SaveRecorder()
        let provider = NSItemProvider()
        provider.suggestedName = "Shared note"
        let importer = makeImporter(recorder: recorder)

        XCTAssertTrue(try importer.importItem(
            "hello from share" as NSString,
            typeIdentifier: UTType.plainText.identifier,
            provider: provider
        ))

        XCTAssertEqual(recorder.files.map(\.displayName), ["Shared note.txt"])
        XCTAssertEqual(recorder.files.first?.contentType, .plainText)
        XCTAssertEqual(String(data: try XCTUnwrap(recorder.files.first?.contents), encoding: .utf8), "hello from share")
    }

    func testTextImportDoesNotDuplicateExistingExtension() throws {
        let recorder = SaveRecorder()
        let provider = NSItemProvider()
        provider.suggestedName = "Already named.txt"
        let importer = makeImporter(recorder: recorder)

        XCTAssertTrue(try importer.importItem(
            "body" as NSString,
            typeIdentifier: UTType.plainText.identifier,
            provider: provider
        ))

        XCTAssertEqual(recorder.files.first?.displayName, "Already named.txt")
    }

    func testFileURLImportPreservesFallbackFilenameAndContents() throws {
        let recorder = SaveRecorder()
        let source = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-drive-share-\(UUID().uuidString)")
            .appendingPathExtension("txt")
        try Data("from files app".utf8).write(to: source)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: source)
        }
        let importer = makeImporter(recorder: recorder)

        XCTAssertTrue(try importer.importItem(
            source as NSURL,
            typeIdentifier: UTType.fileURL.identifier,
            provider: NSItemProvider()
        ))

        XCTAssertEqual(recorder.files.first?.displayName, source.lastPathComponent)
        XCTAssertEqual(recorder.files.first?.contentType, .plainText)
        XCTAssertEqual(String(data: try XCTUnwrap(recorder.files.first?.contents), encoding: .utf8), "from files app")
    }

    func testWebURLImportCreatesUrlFile() throws {
        let recorder = SaveRecorder()
        let importer = makeImporter(recorder: recorder)
        let url = try XCTUnwrap(URL(string: "https://iris.to/share-target"))

        XCTAssertTrue(try importer.importItem(
            url as NSURL,
            typeIdentifier: UTType.url.identifier,
            provider: NSItemProvider()
        ))

        XCTAssertEqual(recorder.files.first?.displayName, "Shared link.url")
        XCTAssertEqual(recorder.files.first?.contentType, .url)
        XCTAssertEqual(String(data: try XCTUnwrap(recorder.files.first?.contents), encoding: .utf8), url.absoluteString)
    }

    func testDataImportUsesSuggestedImageExtension() throws {
        let recorder = SaveRecorder()
        let provider = NSItemProvider()
        provider.suggestedName = "Screenshot from share sheet"
        let importer = makeImporter(recorder: recorder)
        let pngHeader = Data([0x89, 0x50, 0x4E, 0x47])

        XCTAssertTrue(try importer.importItem(
            pngHeader as NSData,
            typeIdentifier: UTType.png.identifier,
            provider: provider
        ))

        XCTAssertEqual(recorder.files.first?.displayName, "Screenshot from share sheet.png")
        XCTAssertEqual(recorder.files.first?.contentType, .png)
        XCTAssertEqual(recorder.files.first?.contents, pngHeader)
    }

    func testUnsupportedItemReturnsFalseWithoutSaving() throws {
        let recorder = SaveRecorder()
        let importer = makeImporter(recorder: recorder)

        XCTAssertFalse(try importer.importItem(
            NSDate(),
            typeIdentifier: UTType.data.identifier,
            provider: NSItemProvider()
        ))

        XCTAssertTrue(recorder.files.isEmpty)
    }

    private func makeImporter(recorder: SaveRecorder? = nil) -> ShareItemImporter {
        ShareItemImporter { displayName, contentType, contents in
            recorder?.files.append(SavedFile(
                displayName: displayName,
                contentType: contentType,
                contents: contents
            ))
        }
    }
}
