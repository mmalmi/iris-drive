import Foundation

extension AppDelegate {
    func scheduleNativeStatusRefresh() {
        guard Thread.isMainThread else {
            DispatchQueue.main.async { [weak self] in
                self?.scheduleNativeStatusRefresh()
            }
            return
        }
        guard runtimePathsForMenu != nil else { return }
        if nativeStatusRefreshInFlight {
            nativeStatusRefreshPending = true
            return
        }
        nativeStatusRefreshInFlight = true
        nativeCoreQueue.async { [weak self] in
            guard let self else { return }
            do {
                try self.applyNativeStateJson(self.desktopCore.refreshJson())
            } catch {
                NSLog("Iris Drive status refresh failed: \(error)")
            }
            DispatchQueue.main.async { [weak self] in
                self?.finishNativeStatusRefresh()
            }
        }
    }

    func finishNativeStatusRefresh() {
        guard Thread.isMainThread else {
            DispatchQueue.main.async { [weak self] in
                self?.finishNativeStatusRefresh()
            }
            return
        }
        nativeStatusRefreshInFlight = false
        if nativeStatusRefreshPending {
            nativeStatusRefreshPending = false
            scheduleNativeStatusRefresh()
        }
    }
}
