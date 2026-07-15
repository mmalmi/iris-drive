import Foundation

struct IrisWebMutableSiteIdentity {
    let npub: String
    let siteName: String
}

func irisWebPublisherNpub(from url: URL?) -> String? {
    irisWebMutableSiteIdentity(from: url)?.npub
}

func irisWebMutableSiteIdentity(from url: URL?) -> IrisWebMutableSiteIdentity? {
    guard let url,
          let host = url.host?.trimmingCharacters(in: .whitespacesAndNewlines),
          !host.isEmpty
    else {
        return nil
    }

    let lowerHost = host.lowercased()
    if lowerHost == "iris.localhost" {
        return irisPortalPathIdentity(from: url)
    }
    guard lowerHost.hasSuffix(".iris.localhost") else { return nil }

    let prefix = String(lowerHost.dropLast(".iris.localhost".count))
    if prefix.isEmpty || prefix == "nhash" || prefix.hasSuffix(".sites") {
        return nil
    }

    let labels = prefix.split(separator: ".").map(String.init)
    guard let npubIndex = labels.lastIndex(where: { $0.hasPrefix("npub1") }),
          npubIndex > 0
    else {
        return nil
    }

    let siteName = labels[..<npubIndex]
        .joined(separator: ".")
        .removingPercentEncoding ?? labels[..<npubIndex].joined(separator: ".")
    return IrisWebMutableSiteIdentity(npub: labels[npubIndex], siteName: siteName)
}

private func irisPortalPathIdentity(from url: URL) -> IrisWebMutableSiteIdentity? {
    let parts = url.pathComponents.filter { $0 != "/" }
    guard parts.count >= 2,
          parts[0].hasPrefix("npub1")
    else {
        return nil
    }
    return IrisWebMutableSiteIdentity(
        npub: parts[0],
        siteName: parts[1].removingPercentEncoding ?? parts[1]
    )
}
