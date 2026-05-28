import SwiftUI

@main
struct IrisDriveIOSApp: App {
    @StateObject private var model = IrisDriveMobileModel()

    var body: some Scene {
        WindowGroup {
            IrisDriveRootView(model: model)
                .onOpenURL { url in
                    model.handle(url: url)
                }
        }
    }
}
