import Foundation

struct IrisDriveUpdateCheck {
    let available: Bool
    let tag: String
    let asset: String

    init(_ result: [String: Any]) throws {
        if let error = result["error"] as? String, !error.isEmpty {
            throw CheckError(message: error)
        }
        guard result["verified"] as? Bool == true,
              let available = result["available"] as? Bool,
              let tag = result["tag"] as? String, !tag.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else { throw CheckError(message: "Could not verify the release announcement") }
        self.available = available
        self.tag = tag
        self.asset = (result["asset"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var statusText: String {
        if !available { return "Newest release seen: \(tag)" }
        return asset.isEmpty ? "Update \(tag) found without a macOS asset" : "Update \(tag) available"
    }

    private struct CheckError: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }
}
