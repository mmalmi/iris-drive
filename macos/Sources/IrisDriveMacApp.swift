import FileProvider
import SwiftUI

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveDisplayName = "Iris Drive"

@main
struct IrisDriveMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        WindowGroup(irisDriveDisplayName) {
            VStack(spacing: 12) {
                Text(irisDriveDisplayName)
                    .font(.title2)
                Text("File Provider domain is registered for local development.")
                    .foregroundStyle(.secondary)
            }
            .frame(width: 420, height: 180)
            .task {
                registerFileProviderDomain()
            }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        registerFileProviderDomain()
    }
}

private func registerFileProviderDomain() {
    let domain = NSFileProviderDomain(
        identifier: irisDriveDomainIdentifier,
        displayName: irisDriveDisplayName
    )
    #if DEBUG
    domain.testingModes = [.alwaysEnabled]
    #endif

    NSFileProviderManager.add(domain) { error in
        if let error {
            NSLog("Iris Drive FileProvider registration failed: \(error)")
        } else {
            NSLog("Iris Drive FileProvider domain registered")
        }
    }
}
