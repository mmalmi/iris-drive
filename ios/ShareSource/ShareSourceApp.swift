import UIKit
import UniformTypeIdentifiers

@main
final class ShareSourceAppDelegate: UIResponder, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        configurationForConnecting session: UISceneSession,
        options: UIScene.ConnectionOptions
    ) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(name: "Share Source", sessionRole: session.role)
        configuration.delegateClass = ShareSourceSceneDelegate.self
        return configuration
    }
}

final class ShareSourceSceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        guard let windowScene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: windowScene)
        window.rootViewController = ShareSourceViewController()
        self.window = window
        window.makeKeyAndVisible()
    }
}

final class ShareSourceViewController: UIViewController {
    private let shareButton = UIButton(type: .system)

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground

        shareButton.setTitle("Share file", for: .normal)
        shareButton.accessibilityIdentifier = "shareFileToIrisDriveButton"
        shareButton.translatesAutoresizingMaskIntoConstraints = false
        shareButton.addTarget(self, action: #selector(shareFile), for: .touchUpInside)
        view.addSubview(shareButton)

        NSLayoutConstraint.activate([
            shareButton.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            shareButton.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])

        if ProcessInfo.processInfo.environment["IRIS_DRIVE_SHARE_SOURCE_AUTOPRESENT"] == "1" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
                self?.shareFile()
            }
        }
    }

    @objc private func shareFile() {
        let environment = ProcessInfo.processInfo.environment
        let filename = environment["IRIS_DRIVE_SHARE_SOURCE_FILENAME"] ?? "Iris Drive Share Sheet Smoke.txt"
        let contents = environment["IRIS_DRIVE_SHARE_SOURCE_CONTENT"] ?? "shared from iOS share sheet\n"
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent(filename, isDirectory: false)

        do {
            if let bytes = environment["IRIS_DRIVE_SHARE_SOURCE_BYTES"].flatMap(Int.init), bytes > 0 {
                try writeLargeFile(to: url, bytes: bytes)
            } else {
                try contents.write(to: url, atomically: true, encoding: .utf8)
            }
        } catch {
            assertionFailure("Unable to write share source file: \(error)")
            return
        }

        let itemProvider = NSItemProvider(contentsOf: url)
            ?? NSItemProvider(item: url as NSURL, typeIdentifier: UTType.fileURL.identifier)
        itemProvider.suggestedName = filename
        let controller = UIActivityViewController(activityItems: [itemProvider], applicationActivities: nil)
        shareButton.accessibilityValue = "Sharing"
        controller.completionWithItemsHandler = { [weak self] _, completed, _, error in
            let outcome = error == nil ? (completed ? "Shared" : "Cancelled") : "Failed"
            self?.shareButton.accessibilityValue = outcome
        }
        controller.popoverPresentationController?.sourceView = shareButton
        controller.popoverPresentationController?.sourceRect = shareButton.bounds
        present(controller, animated: false)
    }

    /// A file as large as a long video, written a megabyte at a time.
    private func writeLargeFile(to url: URL, bytes: Int) throws {
        FileManager.default.createFile(atPath: url.path, contents: nil)
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        var block = [UInt8](repeating: 0, count: 1 << 20)
        var written = 0
        var counter: UInt32 = 0
        while written < bytes {
            for index in stride(from: 0, to: block.count, by: 4) {
                counter &+= 2_654_435_761
                withUnsafeBytes(of: counter) { block.replaceSubrange(index..<index + 4, with: $0) }
            }
            let count = min(block.count, bytes - written)
            try handle.write(contentsOf: block[0..<count])
            written += count
        }
    }
}
