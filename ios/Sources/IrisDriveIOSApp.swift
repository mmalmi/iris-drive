import BackgroundTasks
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
                    model.scheduleBackgroundSyncIfNeeded()
                }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .active {
                        model.startForegroundSyncLoop()
                    } else {
                        model.stopForegroundSyncLoop()
                        model.scheduleBackgroundSyncIfNeeded()
                    }
                }
                .onOpenURL { url in
                    model.handle(url: url)
                }
        }
        .backgroundTask(.appRefresh(IrisDriveBackgroundSyncTask.identifier)) {
            await model.performBackgroundSyncTask()
        }
    }
}
