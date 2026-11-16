import BackgroundTasks
import SwiftUI

@main
struct IrisDriveIOSApp: App {
    @StateObject private var model = IrisDriveMobileModel()
    @Environment(\.scenePhase) private var scenePhase

    init() {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: IrisDriveShareUpload.taskIdentifier, using: nil) { task in
            let upload = IrisDriveShareUpload()
            task.expirationHandler = { upload.cancel() }
            DispatchQueue.global(qos: .utility).async {
                let directory = IrisDriveSharedContainer.baseDirectory.path
                if upload.publish(in: directory, allowNetwork: false) == .pending {
                    // Schedule the next opportunity before working. iOS may suspend this one.
                    do { try IrisDriveShareUpload.schedule() }
                    catch { NSLog("Iris Drive could not reschedule its shared upload: %@", error.localizedDescription) }
                }
                let result = upload.publish(in: directory)
                task.setTaskCompleted(success: result == .complete || result == .paused)
            }
        }
    }

    var body: some Scene {
        WindowGroup {
            IrisDriveRootView(model: model)
                .onAppear {
                    model.ensureFileProviderDomainIfProfileExists()
                    model.handleDebugLaunchEnvironment()
                    model.refreshAfterStartup()
                    model.updateGatewayScenePhase(scenePhase)
                    model.reconcileForegroundWork(isActive: scenePhase == .active)
                    model.scheduleBackgroundSyncIfNeeded()
                }
                .onChange(of: scenePhase) { _, phase in
                    model.updateGatewayScenePhase(phase)
                    model.reconcileForegroundWork(isActive: phase == .active)
                }
                .task(id: scenePhase == .active && model.syncRunning) {
                    guard scenePhase == .active, model.syncRunning else { return }
                    let upload = IrisDriveShareUpload()
                    let directory = IrisDriveSharedContainer.baseDirectory.path
                    await withTaskCancellationHandler {
                        _ = await Task.detached(priority: .utility) { upload.publish(in: directory) }.value
                    } onCancel: { upload.cancel() }
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
