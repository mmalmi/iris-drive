import Foundation

private let nativeFipsStatusMaxRefreshInterval: TimeInterval = 60
private let nativeFipsStatusVolatileKeys: Set<String> = [
    "bytes_recv",
    "bytes_sent",
    "connection_label",
    "packets_recv",
    "packets_sent",
    "srtt_ms",
    "updated_at",
]

extension IrisDriveMobileModel {
    func startNativeFipsStatusDirectoryWatcher(directory: URL) {
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else {
            NSLog("Iris Drive native FIPS status directory watch unavailable: \(directory.path)")
            return
        }
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            guard let self else { return }
            guard self.nativeFipsStatusFileSource == nil else { return }
            if self.startNativeFipsStatusFileWatcher(directory: directory) {
                self.scheduleNativeFipsStatusRefresh()
            }
        }
        source.setCancelHandler { close(descriptor) }
        nativeFipsStatusDirectorySource = source
        source.resume()
    }

    @discardableResult
    func startNativeFipsStatusFileWatcher(directory: URL) -> Bool {
        nativeFipsStatusFileSource?.cancel()
        nativeFipsStatusFileSource = nil
        let statusURL = directory.appendingPathComponent(nativeFipsStatusFileName, isDirectory: false)
        let descriptor = open(statusURL.path, O_EVTONLY)
        guard descriptor >= 0 else { return false }
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .attrib, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            guard let self else { return }
            let data = source.data
            if data.contains(.delete) || data.contains(.rename) {
                self.startNativeFipsStatusFileWatcher(directory: directory)
            }
            self.scheduleNativeFipsStatusRefresh()
        }
        source.setCancelHandler { close(descriptor) }
        nativeFipsStatusFileSource = source
        source.resume()
        return true
    }

    func nativeFipsStatusRefreshIsDue(statusURL: URL, now: Date = Date()) -> Bool {
        let fingerprint = nativeFipsStatusStableFingerprint(statusURL: statusURL)
        let elapsed = now.timeIntervalSince(lastNativeFipsStatusRefreshAt)
        if let fingerprint,
           fingerprint == lastNativeFipsStatusFingerprint,
           elapsed >= 0,
           elapsed < nativeFipsStatusMaxRefreshInterval {
            return false
        }
        if let fingerprint {
            lastNativeFipsStatusFingerprint = fingerprint
        }
        lastNativeFipsStatusRefreshAt = now
        return true
    }

    private func nativeFipsStatusStableFingerprint(statusURL: URL) -> String? {
        guard let data = try? Data(contentsOf: statusURL),
              let object = try? JSONSerialization.jsonObject(with: data)
        else {
            return nil
        }
        let stable = stableNativeFipsStatusValue(object)
        guard JSONSerialization.isValidJSONObject(stable),
              let output = try? JSONSerialization.data(
                  withJSONObject: stable,
                  options: [.sortedKeys]
              )
        else {
            return nil
        }
        return String(data: output, encoding: .utf8)
    }

    private func stableNativeFipsStatusValue(_ value: Any) -> Any {
        if let dictionary = value as? [String: Any] {
            return dictionary.reduce(into: [String: Any]()) { result, entry in
                guard !nativeFipsStatusVolatileKeys.contains(entry.key) else { return }
                result[entry.key] = stableNativeFipsStatusValue(entry.value)
            }
        }
        if let array = value as? [Any] {
            return array.map(stableNativeFipsStatusValue)
        }
        return value
    }
}
