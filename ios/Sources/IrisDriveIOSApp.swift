import SwiftUI

@main
struct IrisDriveIOSApp: App {
    @StateObject private var model = IrisDriveMobileModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            IrisDriveRootView(model: model)
                .onAppear {
                    model.ensureFileProviderDomainIfProfileExists()
                    model.handleDebugLaunchEnvironment()
                    model.startForegroundSyncLoop()
                }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .active {
                        model.startForegroundSyncLoop()
                    } else {
                        model.stopForegroundSyncLoop()
                    }
                }
                .onOpenURL { url in
                    model.handle(url: url)
                }
        }
    }
}
