import SwiftUI

@main
struct IrisDriveIOSApp: App {
    @StateObject private var model = IrisDriveMobileModel()

    var body: some Scene {
        WindowGroup {
            IrisDriveRootView(model: model)
                .onAppear {
                    model.ensureFileProviderDomainIfProfileExists()
                    model.handleDebugLaunchEnvironment()
                }
                .onOpenURL { url in
                    model.handle(url: url)
                }
        }
    }
}
