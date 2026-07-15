import Foundation
import Network

#if DEBUG
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
            var didResume = false

            func finish(_ path: NWPath) {
                guard !didResume else { return }
                didResume = true
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
}
#endif
