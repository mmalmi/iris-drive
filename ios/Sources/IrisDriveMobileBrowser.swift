import Foundation
import WebKit

private let irisNativeBrowserGatewayStatusFileName = "native-browser-gateway-status.json"

struct IrisNativeBrowserGatewayStatus: Decodable {
    var running: Bool?
    var state: String?
    var hashtreeBaseUrl: String?
    var portalUrl: String?
    var proxyPort: UInt16?
    var proxyUrl: String?
    var error: String?
    var embeddedHashtree: [String: String]?

    private enum CodingKeys: String, CodingKey {
        case running
        case state
        case hashtreeBaseUrl = "hashtree_base_url"
        case portalUrl = "portal_url"
        case proxyPort = "proxy_port"
        case proxyUrl = "proxy_url"
        case error
        case embeddedHashtree = "embedded_hashtree"
    }
}

private func irisWebShouldOpenExternally(_ url: URL) -> Bool {
    guard let scheme = url.scheme?.lowercased(),
          scheme == "http" || scheme == "https",
          let host = url.host?.lowercased()
    else {
        return false
    }

    return host == "silent.link"
        || host.hasSuffix(".silent.link")
        || host == "proton.me"
        || host.hasSuffix(".proton.me")
        || host == "protonmail.com"
        || host.hasSuffix(".protonmail.com")
}

func irisWebIsLocalGatewayHost(_ host: String?) -> Bool {
    guard let host = host?.lowercased() else { return false }
    return host == "iris.localhost"
        || host.hasSuffix(".iris.localhost")
        || host == "hash.localhost"
        || host.hasSuffix(".hash.localhost")
}

func irisWebIsTransientGatewayNotFound(_ bodyText: String, url: URL?) -> Bool {
    guard irisWebIsLocalGatewayHost(url?.host) else { return false }
    return bodyText.trimmingCharacters(in: .whitespacesAndNewlines) == "Not found"
}

extension IrisDriveMobileModel {
    func openIrisBrowserWhenReady(_ value: String) {
        guard !isOpeningIrisApps else { return }
        Task { @MainActor [weak self] in
            await self?.openIrisBrowserAfterGatewayReady(value)
        }
    }

    private func openIrisBrowserAfterGatewayReady(_ value: String) async {
        guard localNhashResolverEnabled else {
            statusTitle = "Iris Apps unavailable"
            statusDetail = "Local Iris resolver is disabled."
            return
        }
        guard isSetupComplete else {
            statusTitle = "Iris Apps unavailable"
            statusDetail = "Link this device before opening Iris Apps."
            return
        }
        isOpeningIrisApps = true
        defer { isOpeningIrisApps = false }

        if let url = await readyIrisBrowserURL(for: value) {
            webRoute = IrisWebRoute(url: url)
            return
        }

        statusTitle = "Iris Apps unavailable"
        statusDetail = "Local Iris gateway is still starting."
    }

    private func readyIrisBrowserURL(for value: String) async -> URL? {
        let requested = value.trimmingCharacters(in: .whitespacesAndNewlines)
        for attempt in 0..<40 {
            let portalUrl = activeIrisGatewayPortalURL()
            let source = requested.isEmpty ? portalUrl : requested
            let candidate = localGatewayURL(
                source,
                activePortalUrl: portalUrl
            ).trimmingCharacters(in: .whitespacesAndNewlines)
            if let url = URL(string: candidate),
               URLComponents(url: url, resolvingAgainstBaseURL: false)?.port != nil,
               await localGatewayResponds(portalUrl: portalUrl) {
                return url
            }
            let delay = attempt < 8 ? 250_000_000 : 500_000_000
            do {
                try await Task.sleep(nanoseconds: UInt64(delay))
            } catch {
                return nil
            }
            guard !Task.isCancelled else { return nil }
        }
        return nil
    }

    private func activeIrisGatewayPortalURL() -> String {
        if let status = nativeBrowserGatewayStatus(),
           status.running == true,
           let portalUrl = status.portalUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !portalUrl.isEmpty {
            sitesPortalUrl = portalUrl
            return portalUrl
        }
        return sitesPortalUrl
    }

    private func nativeBrowserGatewayStatus() -> IrisNativeBrowserGatewayStatus? {
        let url = URL(fileURLWithPath: sharedContainerPath, isDirectory: true)
            .appendingPathComponent(irisNativeBrowserGatewayStatusFileName, isDirectory: false)
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(IrisNativeBrowserGatewayStatus.self, from: data)
    }

    func irisNativeHashtreeBaseURL() -> URL? {
        guard let status = nativeBrowserGatewayStatus(),
              status.running == true,
              let value = status.hashtreeBaseUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty
        else {
            return nil
        }
        return URL(string: value)
    }

    private func localGatewayResponds(portalUrl: String) async -> Bool {
        guard let port = URLComponents(string: portalUrl)?.port,
              let url = URL(string: "http://127.0.0.1:\(port)/")
        else {
            return false
        }
        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        request.timeoutInterval = 0.5
        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse else { return true }
            return (100..<600).contains(http.statusCode)
        } catch {
            return false
        }
    }

    func localGatewayURL(_ value: String, activePortalUrl: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var components = URLComponents(string: trimmed),
              let host = components.host,
              let activePort = URLComponents(string: activePortalUrl)?.port
        else {
            return value
        }
        let lowerHost = host.lowercased()
        let isLocalGatewayHost = lowerHost == "iris.localhost"
            || lowerHost.hasSuffix(".iris.localhost")
            || lowerHost == "hash.localhost"
            || lowerHost.hasSuffix(".hash.localhost")
        guard isLocalGatewayHost else { return value }
        components.port = activePort
        return components.url?.absoluteString ?? value
    }

    func irisWebNavigationAction(for url: URL) -> IrisWebNavigationAction {
        let classification = IrisDriveNativeLinkInput.classify(url.absoluteString)
        switch classification.kind {
        case "iris_web":
            let localOpenURL = localGatewayURL(classification.localOpenUrl)
            if let localURL = URL(string: localOpenURL),
               localURL.absoluteString != url.absoluteString {
                return .redirect(localURL)
            }
            return .allow
        case "share_dialog", "nhash_file", "mutable_file", "invite", "app_key_approval":
            return .handleNative(url)
        default:
            if irisWebShouldOpenExternally(url) {
                return .openExternal(url)
            }
            if let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https" {
                return .allow
            }
            return .cancel
        }
    }

    func configureIrisWebDataStore(_ dataStore: WKWebsiteDataStore) {
        dataStore.proxyConfigurations = irisWebProxyConfigurations()
    }

    private func irisWebProxyConfigurations() -> [ProxyConfiguration] {
        guard let status = nativeBrowserGatewayStatus(),
              status.running == true,
              let proxyPort = status.proxyPort,
              let port = NWEndpoint.Port(rawValue: proxyPort)
        else {
            return []
        }
        var proxy = ProxyConfiguration(
            httpCONNECTProxy: .hostPort(host: "127.0.0.1", port: port)
        )
        proxy.allowFailover = false
        proxy.matchDomains = [
            "iris.localhost",
            "hash.localhost",
        ]
        return [proxy]
    }
}

enum IrisWebNavigationAction {
    case allow
    case redirect(URL)
    case handleNative(URL)
    case openExternal(URL)
    case cancel
}

#if DEBUG
private struct IrisDebugWebViewProbeResult {
    var loaded: Bool
    var elapsedMs: Int
    var finalURL: String
    var title: String
    var bodyText: String
    var readyState: String
    var htmlLength: Int
    var htmlPrefix: String
    var diagnosticsJson: String
    var screenshotPath: String
    var screenshotError: String
    var error: String
    var errorDomain: String
    var errorCode: Int
}

private struct IrisDebugHTTPProbeResult {
    var ok: Bool
    var statusCode: Int
    var error: String
    var errorDomain: String
    var errorCode: Int
}

private struct IrisDebugTextHTTPProbeResult {
    var ok: Bool
    var statusCode: Int
    var bodyText: String
    var error: String
    var errorDomain: String
    var errorCode: Int
}

struct IrisDebugNetworkPathResult {
    var status: String
    var unsatisfiedReason: String
    var availableInterfaces: [String]
    var usesWifi: Bool
    var usesCellular: Bool
    var usesWiredEthernet: Bool
    var usesLoopback: Bool
    var usesOther: Bool
    var isExpensive: Bool
    var isConstrained: Bool
    var supportsDNS: Bool
    var supportsIPv4: Bool
    var supportsIPv6: Bool
}

@MainActor
private final class IrisDebugWebViewProbe: NSObject, WKNavigationDelegate {
    private weak var model: IrisDriveMobileModel?
    private var webView: WKWebView?
    private var window: UIWindow?
    private var continuation: CheckedContinuation<IrisDebugWebViewProbeResult, Never>?
    private var started = Date()
    private var finished = false
    private var transientNotFoundReloads = 0
    private var timeoutTask: Task<Void, Never>?

    init(model: IrisDriveMobileModel) {
        self.model = model
    }

    func load(_ url: URL, timeoutMs: UInt64) async -> IrisDebugWebViewProbeResult {
        started = Date()
        finished = false
        transientNotFoundReloads = 0
        let frame = UIScreen.main.bounds.isEmpty
            ? CGRect(x: 0, y: 0, width: 390, height: 844)
            : UIScreen.main.bounds
        let configuration = WKWebViewConfiguration()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        configuration.userContentController = WKUserContentController()
        model?.configureIrisWebDataStore(configuration.websiteDataStore)
        let webView = WKWebView(frame: frame, configuration: configuration)
        webView.navigationDelegate = self
        let viewController = UIViewController()
        viewController.view.backgroundColor = .systemBackground
        viewController.view.frame = frame
        webView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        viewController.view.addSubview(webView)
        let windowScene = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first { $0.activationState == .foregroundActive }
            ?? UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = windowScene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: frame)
        window.frame = frame
        window.rootViewController = viewController
        window.windowLevel = .alert + 1
        window.makeKeyAndVisible()
        self.window = window
        self.webView = webView
        return await withCheckedContinuation { continuation in
            self.continuation = continuation
            self.timeoutTask = Task { @MainActor [weak self] in
                try? await Task.sleep(nanoseconds: timeoutMs * 1_000_000)
                await self?.finishFromCurrentPage(
                    loaded: false,
                    error: "Timed out waiting for WKWebView to finish loading",
                    nsError: nil
                )
            }
            webView.load(URLRequest(url: url))
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        Task { @MainActor [weak self, weak webView] in
            let settleMs = UInt64(
                ProcessInfo.processInfo.environment["IRIS_DRIVE_DEBUG_WEBVIEW_SETTLE_MS"] ?? ""
            ) ?? 6_000
            try? await Task.sleep(nanoseconds: settleMs * 1_000_000)
            guard let self, let webView else { return }
            let bodyText = await self.evaluateString(
                webView,
                "document.body ? document.body.innerText : ''"
            )
            if irisWebIsTransientGatewayNotFound(bodyText, url: webView.url),
               self.transientNotFoundReloads < 8 {
                self.transientNotFoundReloads += 1
                webView.reload()
                return
            }
            await self.finishFromCurrentPage(loaded: true, error: "", nsError: nil)
        }
    }

    private func finishFromCurrentPage(loaded: Bool, error: String, nsError: NSError?) async {
        guard !finished, let webView else { return }
        let title = await evaluateString(webView, "document.title")
        let bodyText = await evaluateString(
            webView,
            "document.body ? document.body.innerText : ''"
        )
        let readyState = await evaluateString(webView, "document.readyState")
        let htmlLength = await evaluateInt(
            webView,
            "document.documentElement ? document.documentElement.outerHTML.length : 0"
        )
        let htmlPrefix = await evaluateString(
            webView,
            "document.documentElement ? document.documentElement.outerHTML.slice(0, 8000) : ''"
        )
        let diagnosticsJson = await evaluateString(
            webView,
            """
            JSON.stringify({
              bodyChildCount: document.body ? document.body.children.length : -1,
              rootChildCount: document.getElementById('root') ? document.getElementById('root').children.length : -1,
              appChildCount: document.getElementById('app') ? document.getElementById('app').children.length : -1,
              scripts: Array.from(document.scripts || []).map(s => s.src || s.textContent.slice(0, 80)).slice(0, 40),
              stylesheets: Array.from(document.querySelectorAll('link[rel="stylesheet"]')).map(l => l.href).slice(0, 40),
              resources: performance.getEntriesByType('resource').map(r => ({
                name: r.name,
                transferSize: r.transferSize || 0,
                encodedBodySize: r.encodedBodySize || 0,
                decodedBodySize: r.decodedBodySize || 0
              })).slice(0, 80)
            })
            """
        )
        let screenshot = await writeSnapshot(webView)
        finish(
            loaded: loaded,
            title: title,
            bodyText: bodyText,
            readyState: readyState,
            htmlLength: htmlLength,
            htmlPrefix: htmlPrefix,
            diagnosticsJson: diagnosticsJson,
            screenshotPath: screenshot.path,
            screenshotError: screenshot.error,
            finalURL: webView.url?.absoluteString ?? "",
            error: error,
            nsError: nsError
        )
    }

    private func evaluateString(_ webView: WKWebView, _ script: String) async -> String {
        await withCheckedContinuation { continuation in
            webView.evaluateJavaScript(script) { value, _ in
                continuation.resume(returning: value as? String ?? "")
            }
        }
    }

    private func evaluateInt(_ webView: WKWebView, _ script: String) async -> Int {
        await withCheckedContinuation { continuation in
            webView.evaluateJavaScript(script) { value, _ in
                continuation.resume(returning: value as? Int ?? 0)
            }
        }
    }

    private func writeSnapshot(_ webView: WKWebView) async -> (path: String, error: String) {
        await withCheckedContinuation { continuation in
            webView.takeSnapshot(with: nil) { image, error in
                if let error {
                    continuation.resume(returning: ("", (error as NSError).localizedDescription))
                    return
                }
                guard let data = image?.pngData() else {
                    continuation.resume(returning: ("", "Snapshot produced no PNG data"))
                    return
                }
                let url = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)
                    .first?
                    .appendingPathComponent("debug-iris-apps-webview.png", isDirectory: false)
                guard let url else {
                    continuation.resume(returning: ("", "Documents directory unavailable"))
                    return
                }
                do {
                    try data.write(to: url, options: [.atomic])
                    continuation.resume(returning: (url.path, ""))
                } catch {
                    continuation.resume(returning: ("", (error as NSError).localizedDescription))
                }
            }
        }
    }

    func webView(
        _ webView: WKWebView,
        didFail navigation: WKNavigation!,
        withError error: Error
    ) {
        finish(error)
    }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        finish(error)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        finish(
            loaded: false,
            error: "Web content process terminated",
            nsError: nil
        )
    }

    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.allow)
            return
        }
        switch model?.irisWebNavigationAction(for: url) ?? .allow {
        case .allow:
            decisionHandler(.allow)
        case .redirect(let localURL):
            webView.load(URLRequest(url: localURL))
            decisionHandler(.cancel)
        case .handleNative(let nativeURL):
            finish(
                loaded: false,
                error: "Unexpected native Iris navigation during WebView probe: \(nativeURL.absoluteString)",
                nsError: nil
            )
            decisionHandler(.cancel)
        case .openExternal(let externalURL):
            finish(
                loaded: false,
                error: "Unexpected external navigation during WebView probe: \(externalURL.absoluteString)",
                nsError: nil
            )
            decisionHandler(.cancel)
        case .cancel:
            finish(
                loaded: false,
                error: "Navigation was cancelled by Iris WebView policy: \(url.absoluteString)",
                nsError: nil
            )
            decisionHandler(.cancel)
        }
    }

    private func finish(_ error: Error) {
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain,
           nsError.code == NSURLErrorCancelled {
            return
        }
        finish(
            loaded: false,
            error: nsError.localizedDescription,
            nsError: nsError
        )
    }

    private func finish(
        loaded: Bool,
        title: String = "",
        bodyText: String = "",
        readyState: String = "",
        htmlLength: Int = 0,
        htmlPrefix: String = "",
        diagnosticsJson: String = "",
        screenshotPath: String = "",
        screenshotError: String = "",
        finalURL: String? = nil,
        error: String,
        nsError: NSError?
    ) {
        guard !finished else { return }
        finished = true
        timeoutTask?.cancel()
        let result = IrisDebugWebViewProbeResult(
            loaded: loaded,
            elapsedMs: Int(Date().timeIntervalSince(started) * 1000),
            finalURL: finalURL ?? webView?.url?.absoluteString ?? "",
            title: title,
            bodyText: bodyText,
            readyState: readyState,
            htmlLength: htmlLength,
            htmlPrefix: htmlPrefix,
            diagnosticsJson: diagnosticsJson,
            screenshotPath: screenshotPath,
            screenshotError: screenshotError,
            error: error,
            errorDomain: nsError?.domain ?? "",
            errorCode: nsError?.code ?? 0
        )
        continuation?.resume(returning: result)
        continuation = nil
        webView?.navigationDelegate = nil
        webView = nil
        window?.isHidden = true
        window = nil
    }
}

extension IrisDriveMobileModel {
    func debugProbeIrisApps() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            let environment = ProcessInfo.processInfo.environment
            if environment["IRIS_DRIVE_DEBUG_RESET_LOCAL_STATE"] == "1" {
                guard self.allowDebugStateMutation(
                    action: "probe-iris-apps reset-local-state",
                    environment: environment
                ) else {
                    return
                }
                resetLocalState()
            }
            if !hasLocalProfile {
                guard self.allowDebugStateMutation(
                    action: "probe-iris-apps create-profile",
                    environment: environment
                ) else {
                    return
                }
                createProfile(
                    username: environment["IRIS_DRIVE_DEBUG_USERNAME"] ?? "",
                    profilePhotoName: ""
                )
            }

            let started = Date()
            let timeoutMilliseconds = UInt64(
                environment["IRIS_DRIVE_DEBUG_PROBE_TIMEOUT_MS"] ?? ""
            ) ?? 10_000
            let timeout = timeoutMilliseconds * 1_000_000
            let openTask = Task { @MainActor in
                await self.openIrisBrowserAfterGatewayReady(self.sitesPortalUrl)
                return self.webRoute != nil
            }
            let timeoutTask = Task { () -> Bool in
                do {
                    try await Task.sleep(nanoseconds: timeout)
                } catch {
                    return false
                }
                return false
            }
            let opened = await withTaskGroup(of: Bool.self) { group -> Bool in
                group.addTask { await openTask.value }
                group.addTask { await timeoutTask.value }
                let result = await group.next() ?? false
                openTask.cancel()
                timeoutTask.cancel()
                group.cancelAll()
                return result
            }
            let routeReadyMs = Int(Date().timeIntervalSince(started) * 1000)
            let routeURL = webRoute?.url
            var routeHTTPResult = IrisDebugHTTPProbeResult(
                ok: false,
                statusCode: 0,
                error: "HTTP probe skipped",
                errorDomain: "",
                errorCode: 0
            )
            var routeProxyHTTPResult = routeHTTPResult
            let webViewResult: IrisDebugWebViewProbeResult
            if let routeURL {
                webViewResult = await IrisDebugWebViewProbe(model: self)
                    .load(routeURL, timeoutMs: timeoutMilliseconds)
                if environment["IRIS_DRIVE_DEBUG_PROBE_HTTP"] == "1" {
                    routeHTTPResult = await debugProbeHTTP(routeURL, useIrisProxy: false)
                    routeProxyHTTPResult = await debugProbeHTTP(routeURL, useIrisProxy: true)
                }
            } else {
                routeHTTPResult = IrisDebugHTTPProbeResult(
                    ok: false,
                    statusCode: 0,
                    error: "Iris Apps did not produce a browser route",
                    errorDomain: "",
                    errorCode: 0
                )
                routeProxyHTTPResult = routeHTTPResult
                webViewResult = IrisDebugWebViewProbeResult(
                    loaded: false,
                    elapsedMs: 0,
                    finalURL: "",
                    title: "",
                    bodyText: "",
                    readyState: "",
                    htmlLength: 0,
                    htmlPrefix: "",
                    diagnosticsJson: "",
                    screenshotPath: "",
                    screenshotError: "",
                    error: "Iris Apps did not produce a browser route",
                    errorDomain: "",
                    errorCode: 0
                )
            }
            let elapsedMs = Int(Date().timeIntervalSince(started) * 1000)
            let userVisibleElapsedMs = routeReadyMs + webViewResult.elapsedMs
            let gatewayStatus = nativeBrowserGatewayStatus()
            let hashtreeStatusResult = await debugProbeTextHTTP(
                gatewayStatus?.hashtreeBaseUrl.map { "\($0.trimmingCharacters(in: .whitespacesAndNewlines))/api/status" } ?? ""
            )
            let embeddedRouteResult = await debugProbeTextHTTP(
                debugEmbeddedHashtreeRouteURL(gatewayStatus: gatewayStatus, routeURL: routeURL)
            )
            let embeddedResolveResult = await debugProbeTextHTTP(
                debugEmbeddedHashtreeResolveURL(
                    gatewayStatus: gatewayStatus,
                    routeURL: routeURL,
                    refresh: true
                )
            )
            let embeddedCachedResolveResult = await debugProbeTextHTTP(
                debugEmbeddedHashtreeResolveURL(
                    gatewayStatus: gatewayStatus,
                    routeURL: routeURL,
                    refresh: false
                )
            )
            let embeddedSettingsText = debugReadEmbeddedBrowserSettings(gatewayStatus)
            var uploadRootHTTPResult = IrisDebugHTTPProbeResult(
                ok: false,
                statusCode: 0,
                error: "URL invalid",
                errorDomain: "",
                errorCode: 0
            )
            if let uploadRootURL = URL(string: "https://upload.iris.to/") {
                uploadRootHTTPResult = await debugProbeHTTP(uploadRootURL, useIrisProxy: false)
            }
            let networkPathResult = await debugNetworkPathSnapshot()
            writeIrisAppsProbeResult([
                "probe_id": environment["IRIS_DRIVE_DEBUG_PROBE_ID"] ?? "",
                "opened": opened,
                "elapsed_ms": elapsedMs,
                "route_ready_ms": routeReadyMs,
                "user_visible_elapsed_ms": userVisibleElapsedMs,
                "route_url": routeURL?.absoluteString ?? "",
                "route_host": routeURL?.host ?? "",
                "route_port": routeURL?.port ?? 0,
                "route_http_ok": routeHTTPResult.ok,
                "route_http_status_code": routeHTTPResult.statusCode,
                "route_http_error": routeHTTPResult.error,
                "route_http_error_domain": routeHTTPResult.errorDomain,
                "route_http_error_code": routeHTTPResult.errorCode,
                "route_proxy_http_ok": routeProxyHTTPResult.ok,
                "route_proxy_http_status_code": routeProxyHTTPResult.statusCode,
                "route_proxy_http_error": routeProxyHTTPResult.error,
                "route_proxy_http_error_domain": routeProxyHTTPResult.errorDomain,
                "route_proxy_http_error_code": routeProxyHTTPResult.errorCode,
                "webview_loaded": webViewResult.loaded,
                "webview_elapsed_ms": webViewResult.elapsedMs,
                "webview_final_url": webViewResult.finalURL,
                "webview_title": webViewResult.title,
                "webview_body_text": String(webViewResult.bodyText.prefix(8_000)),
                "webview_ready_state": webViewResult.readyState,
                "webview_html_length": webViewResult.htmlLength,
                "webview_html_prefix": String(webViewResult.htmlPrefix.prefix(8_000)),
                "webview_diagnostics_json": String(webViewResult.diagnosticsJson.prefix(8_000)),
                "webview_screenshot_path": webViewResult.screenshotPath,
                "webview_screenshot_error": webViewResult.screenshotError,
                "webview_error": webViewResult.error,
                "webview_error_domain": webViewResult.errorDomain,
                "webview_error_code": webViewResult.errorCode,
                "status_title": statusTitle,
                "status_detail": statusDetail,
                "setup_complete": isSetupComplete,
                "sites_portal_url_present": !sitesPortalUrl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                "web_route_present": webRoute != nil,
                "gateway_status_present": gatewayStatus != nil,
                "gateway_status_running": gatewayStatus?.running ?? false,
                "gateway_status_state": gatewayStatus?.state ?? "",
                "gateway_status_hashtree_base_url": gatewayStatus?.hashtreeBaseUrl ?? "",
                "gateway_status_portal_url": gatewayStatus?.portalUrl ?? "",
                "gateway_status_error": gatewayStatus?.error ?? "",
                "gateway_status_embedded_hashtree": gatewayStatus?.embeddedHashtree ?? [:],
                "gateway_status_embedded_browser_settings": String(embeddedSettingsText.prefix(8_000)),
                "hashtree_status_ok": hashtreeStatusResult.ok,
                "hashtree_status_code": hashtreeStatusResult.statusCode,
                "hashtree_status_body": String(hashtreeStatusResult.bodyText.prefix(8_000)),
                "hashtree_status_error": hashtreeStatusResult.error,
                "hashtree_status_error_domain": hashtreeStatusResult.errorDomain,
                "hashtree_status_error_code": hashtreeStatusResult.errorCode,
                "embedded_route_http_ok": embeddedRouteResult.ok,
                "embedded_route_http_status_code": embeddedRouteResult.statusCode,
                "embedded_route_http_body": String(embeddedRouteResult.bodyText.prefix(8_000)),
                "embedded_route_http_error": embeddedRouteResult.error,
                "embedded_route_http_error_domain": embeddedRouteResult.errorDomain,
                "embedded_route_http_error_code": embeddedRouteResult.errorCode,
                "embedded_resolve_http_ok": embeddedResolveResult.ok,
                "embedded_resolve_http_status_code": embeddedResolveResult.statusCode,
                "embedded_resolve_http_body": String(embeddedResolveResult.bodyText.prefix(8_000)),
                "embedded_resolve_http_error": embeddedResolveResult.error,
                "embedded_resolve_http_error_domain": embeddedResolveResult.errorDomain,
                "embedded_resolve_http_error_code": embeddedResolveResult.errorCode,
                "embedded_cached_resolve_http_ok": embeddedCachedResolveResult.ok,
                "embedded_cached_resolve_http_status_code": embeddedCachedResolveResult.statusCode,
                "embedded_cached_resolve_http_body": String(embeddedCachedResolveResult.bodyText.prefix(8_000)),
                "embedded_cached_resolve_http_error": embeddedCachedResolveResult.error,
                "embedded_cached_resolve_http_error_domain": embeddedCachedResolveResult.errorDomain,
                "embedded_cached_resolve_http_error_code": embeddedCachedResolveResult.errorCode,
                "upload_root_http_ok": uploadRootHTTPResult.ok,
                "upload_root_http_status_code": uploadRootHTTPResult.statusCode,
                "upload_root_http_error": uploadRootHTTPResult.error,
                "upload_root_http_error_domain": uploadRootHTTPResult.errorDomain,
                "upload_root_http_error_code": uploadRootHTTPResult.errorCode,
                "network_path_status": networkPathResult.status,
                "network_path_unsatisfied_reason": networkPathResult.unsatisfiedReason,
                "network_path_available_interfaces": networkPathResult.availableInterfaces,
                "network_path_uses_wifi": networkPathResult.usesWifi,
                "network_path_uses_cellular": networkPathResult.usesCellular,
                "network_path_uses_wired_ethernet": networkPathResult.usesWiredEthernet,
                "network_path_uses_loopback": networkPathResult.usesLoopback,
                "network_path_uses_other": networkPathResult.usesOther,
                "network_path_is_expensive": networkPathResult.isExpensive,
                "network_path_is_constrained": networkPathResult.isConstrained,
                "network_path_supports_dns": networkPathResult.supportsDNS,
                "network_path_supports_ipv4": networkPathResult.supportsIPv4,
                "network_path_supports_ipv6": networkPathResult.supportsIPv6,
            ])
        }
    }

    private func debugProbeHTTP(_ url: URL, useIrisProxy: Bool) async -> IrisDebugHTTPProbeResult {
        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        request.timeoutInterval = 2
        let session: URLSession
        if useIrisProxy {
            let configuration = URLSessionConfiguration.ephemeral
            configuration.proxyConfigurations = irisWebProxyConfigurations()
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

    private func debugProbeTextHTTP(_ value: String) async -> IrisDebugTextHTTPProbeResult {
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
        request.timeoutInterval = 3
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

    private func writeIrisAppsProbeResult(_ value: [String: Any]) {
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
