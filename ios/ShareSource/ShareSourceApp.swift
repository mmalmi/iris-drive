import UIKit
import UniformTypeIdentifiers

@main
final class ShareSourceAppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        let window = UIWindow(frame: UIScreen.main.bounds)
        window.rootViewController = ShareSourceViewController()
        window.makeKeyAndVisible()
        self.window = window
        return true
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
            try contents.write(to: url, atomically: true, encoding: .utf8)
        } catch {
            assertionFailure("Unable to write share source file: \(error)")
            return
        }

        let itemProvider = NSItemProvider(contentsOf: url)
            ?? NSItemProvider(item: url as NSURL, typeIdentifier: UTType.fileURL.identifier)
        itemProvider.suggestedName = filename
        let controller = UIActivityViewController(activityItems: [itemProvider], applicationActivities: nil)
        controller.popoverPresentationController?.sourceView = shareButton
        controller.popoverPresentationController?.sourceRect = shareButton.bounds
        present(controller, animated: false)
    }
}
