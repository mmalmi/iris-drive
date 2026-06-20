import Foundation

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
            await refreshInBackground()
            let source = requested.isEmpty ? sitesPortalUrl : requested
            let candidate = localGatewayURL(source).trimmingCharacters(in: .whitespacesAndNewlines)
            if let url = URL(string: candidate),
               URLComponents(url: url, resolvingAgainstBaseURL: false)?.port != nil {
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
}
