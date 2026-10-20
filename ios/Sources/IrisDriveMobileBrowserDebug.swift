import Foundation
import Network
import os

#if DEBUG
struct IrisDebugHTTPProbeResult {
    var ok: Bool
    var statusCode: Int
    var error: String
    var errorDomain: String
    var errorCode: Int
}

struct IrisDebugTextHTTPProbeResult {
    var ok: Bool
    var statusCode: Int
    var bodyText: String
    var error: String
    var errorDomain: String
    var errorCode: Int
}

@MainActor
final class IrisDebugProbeCompletion {
    private var timeoutTask: Task<Void, Never>?
    private var onTimeout: (() -> Void)?
    private(set) var completed = false
    private(set) var fields: [String: Any] = [:]

    func record(_ fields: [String: Any]) {
        guard !completed else { return }
        self.fields.merge(fields) { _, current in current }
    }

    func start(timeoutMilliseconds: UInt64, onTimeout: @escaping () -> Void) {
        precondition(timeoutTask == nil && !completed)
        self.onTimeout = onTimeout
        timeoutTask = Task { @MainActor [weak self] in
            do {
                try await Task.sleep(nanoseconds: timeoutMilliseconds * 1_000_000)
            } catch {
                return
            }
            self?.deadlineExpired()
        }
    }

    func deadlineExpired() {
        guard let onTimeout else { return }
        complete(onTimeout)
    }

    @discardableResult
    func complete(_ publish: () -> Void) -> Bool {
        guard !completed else { return false }
        completed = true
        timeoutTask?.cancel()
        timeoutTask = nil
        onTimeout = nil
        publish()
        return true
    }
}

extension IrisDriveMobileModel {
    func debugEmbeddedHashtreeRouteURL(
        gatewayStatus: IrisNativeBrowserGatewayStatus?,
        routeURL: URL?
    ) -> String {
        guard let baseURL = gatewayStatus?.hashtreeBaseUrl?
                .trimmingCharacters(in: .whitespacesAndNewlines),
              !baseURL.isEmpty,
              let host = routeURL?.host?.lowercased()
        else {
            return ""
        }
        let suffix = ".iris.localhost"
        guard host.hasSuffix(suffix) else {
            return ""
        }
        let labels = String(host.dropLast(suffix.count)).split(separator: ".")
        guard labels.count >= 2 else {
            return ""
        }
        let tree = labels[0]
        let npub = labels[1]
        return "\(baseURL)/htree/\(npub)/\(tree)/index.html"
    }

    func debugEmbeddedHashtreeResolveURL(
        gatewayStatus: IrisNativeBrowserGatewayStatus?,
        routeURL: URL?,
        refresh: Bool
    ) -> String {
        guard let baseURL = gatewayStatus?.hashtreeBaseUrl?
                .trimmingCharacters(in: .whitespacesAndNewlines),
              !baseURL.isEmpty,
              let host = routeURL?.host?.lowercased()
        else {
            return ""
        }
        let suffix = ".iris.localhost"
        guard host.hasSuffix(suffix) else {
            return ""
        }
        let labels = String(host.dropLast(suffix.count)).split(separator: ".")
        guard labels.count >= 2 else {
            return ""
        }
        let tree = labels[0]
        let npub = labels[1]
        let querySuffix = refresh ? "?refresh=1" : ""
        return "\(baseURL)/api/resolve/\(npub)/\(tree)\(querySuffix)"
    }

    func debugReadEmbeddedBrowserSettings(_ status: IrisNativeBrowserGatewayStatus?) -> String {
        guard let configDir = status?.embeddedHashtree?["config_dir"],
              !configDir.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            return ""
        }
        let settingsURL = URL(fileURLWithPath: configDir, isDirectory: true)
            .appendingPathComponent("browser_settings.json", isDirectory: false)
        return (try? String(contentsOf: settingsURL, encoding: .utf8)) ?? ""
    }

    nonisolated func debugNetworkPathSnapshot() async -> IrisDebugNetworkPathResult {
        await withCheckedContinuation { continuation in
            let monitor = NWPathMonitor()
            let queue = DispatchQueue(label: "to.iris.drive.debug.network-path")
            let didResume = OSAllocatedUnfairLock(initialState: false)

            @Sendable func finish(_ path: NWPath) {
                let isFirst = didResume.withLock { resumed in
                    defer { resumed = true }
                    return !resumed
                }
                guard isFirst else { return }
                monitor.cancel()
                continuation.resume(returning: debugNetworkPathResult(from: path))
            }

            monitor.pathUpdateHandler = { path in
                finish(path)
            }
            monitor.start(queue: queue)
            queue.asyncAfter(deadline: .now() + .milliseconds(1_500)) {
                finish(monitor.currentPath)
            }
        }
    }

    nonisolated private func debugNetworkPathResult(from path: NWPath) -> IrisDebugNetworkPathResult {
        IrisDebugNetworkPathResult(
            status: debugNetworkPathStatus(path.status),
            unsatisfiedReason: debugNetworkPathUnsatisfiedReason(path.unsatisfiedReason),
            availableInterfaces: path.availableInterfaces.map { debugNetworkInterfaceType($0.type) },
            usesWifi: path.usesInterfaceType(.wifi),
            usesCellular: path.usesInterfaceType(.cellular),
            usesWiredEthernet: path.usesInterfaceType(.wiredEthernet),
            usesLoopback: path.usesInterfaceType(.loopback),
            usesOther: path.usesInterfaceType(.other),
            isExpensive: path.isExpensive,
            isConstrained: path.isConstrained,
            supportsDNS: path.supportsDNS,
            supportsIPv4: path.supportsIPv4,
            supportsIPv6: path.supportsIPv6
        )
    }

    nonisolated private func debugNetworkPathStatus(_ status: NWPath.Status) -> String {
        switch status {
        case .satisfied:
            return "satisfied"
        case .unsatisfied:
            return "unsatisfied"
        case .requiresConnection:
            return "requires_connection"
        @unknown default:
            return "unknown"
        }
    }

    nonisolated private func debugNetworkPathUnsatisfiedReason(_ reason: NWPath.UnsatisfiedReason) -> String {
        switch reason {
        case .notAvailable:
            return "not_available"
        case .cellularDenied:
            return "cellular_denied"
        case .wifiDenied:
            return "wifi_denied"
        case .localNetworkDenied:
            return "local_network_denied"
        case .vpnInactive:
            return "vpn_inactive"
        @unknown default:
            return "unknown"
        }
    }

    nonisolated private func debugNetworkInterfaceType(_ type: NWInterface.InterfaceType) -> String {
        switch type {
        case .wifi:
            return "wifi"
        case .cellular:
            return "cellular"
        case .wiredEthernet:
            return "wired_ethernet"
        case .loopback:
            return "loopback"
        case .other:
            return "other"
        @unknown default:
            return "unknown"
        }
    }
    func debugProbeHTTP(_ url: URL, useIrisProxy: Bool) async -> IrisDebugHTTPProbeResult {
        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        request.timeoutInterval = 2
        let session: URLSession
        if useIrisProxy {
            let configuration = URLSessionConfiguration.ephemeral
            configuration.proxyConfigurations = debugIrisWebProxyConfigurations()
            session = URLSession(configuration: configuration)
        } else {
            session = URLSession.shared
        }
        do {
            let (_, response) = try await session.data(for: request)
            let statusCode = (response as? HTTPURLResponse)?.statusCode ?? 0
            return IrisDebugHTTPProbeResult(
                ok: statusCode == 0 || (100..<600).contains(statusCode),
                statusCode: statusCode,
                error: "",
                errorDomain: "",
                errorCode: 0
            )
        } catch {
            let nsError = error as NSError
            return IrisDebugHTTPProbeResult(
                ok: false,
                statusCode: 0,
                error: nsError.localizedDescription,
                errorDomain: nsError.domain,
                errorCode: nsError.code
            )
        }
    }

    func debugProbeTextHTTP(
        _ value: String,
        timeout: TimeInterval = 3
    ) async -> IrisDebugTextHTTPProbeResult {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: trimmed), !trimmed.isEmpty else {
            return IrisDebugTextHTTPProbeResult(
                ok: false,
                statusCode: 0,
                bodyText: "",
                error: "URL missing",
                errorDomain: "",
                errorCode: 0
            )
        }
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        request.timeoutInterval = timeout
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            let statusCode = (response as? HTTPURLResponse)?.statusCode ?? 0
            return IrisDebugTextHTTPProbeResult(
                ok: statusCode == 0 || (100..<600).contains(statusCode),
                statusCode: statusCode,
                bodyText: String(data: data, encoding: .utf8) ?? "",
                error: "",
                errorDomain: "",
                errorCode: 0
            )
        } catch {
            let nsError = error as NSError
            return IrisDebugTextHTTPProbeResult(
                ok: false,
                statusCode: 0,
                bodyText: "",
                error: nsError.localizedDescription,
                errorDomain: nsError.domain,
                errorCode: nsError.code
            )
        }
    }

    func writeIrisAppsProbeResult(_ value: [String: Any]) {
        guard JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
        else { return }
        let destinations = [
            IrisDriveSharedContainer.baseDirectory
                .appendingPathComponent("debug-iris-apps-probe.json", isDirectory: false),
            FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first?
                .appendingPathComponent("debug-iris-apps-probe.json", isDirectory: false),
        ].compactMap { $0 }
        for url in destinations {
            try? FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try? data.write(to: url, options: [.atomic])
        }
    }
}
#endif
