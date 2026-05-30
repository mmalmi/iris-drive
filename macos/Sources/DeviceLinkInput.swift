import Foundation

extension IrisDriveControlPanel {
    func submitSetupOwner(_ value: String, force: Bool) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              force || isCompleteDeviceLinkOwnerInput(trimmed),
              submittedSetupOwner != trimmed else { return }
        submittedSetupOwner = trimmed
        controller.linkDevice(owner: trimmed)
    }
}

func isCompleteDeviceLinkOwnerInput(_ value: String) -> Bool {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.contains(where: { $0.isWhitespace }) else { return false }
    let lower = trimmed.lowercased()
    if lower.hasPrefix("npub1") {
        return lower.count >= 63
    }
    if lower.count == 64, lower.unicodeScalars.allSatisfy(isAsciiHexDigit) {
        return true
    }
    for prefix in [
        "iris-drive://invite/",
        "iris-drive:/invite/",
        "https://drive.iris.to/invite/",
    ] where lower.hasPrefix(prefix) {
        return lower.dropFirst(prefix.count).count >= 32
    }
    if lower.hasPrefix("iris-drive://link-device?")
        || lower.hasPrefix("iris-drive:/link-device?")
        || lower.hasPrefix("https://drive.iris.to/link-device?") {
        return lower.contains("owner=") && lower.contains("admin=") && lower.contains("secret=")
    }
    return false
}

private func isAsciiHexDigit(_ scalar: Unicode.Scalar) -> Bool {
    (48...57).contains(scalar.value) || (97...102).contains(scalar.value)
}
