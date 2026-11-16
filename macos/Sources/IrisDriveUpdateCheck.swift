import Foundation
import SwiftUI

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

/// One step of an update install, worded by the core. The app only renders it.
struct IrisDriveUpdateProgress: Equatable {
    var stage = "idle"
    var text = ""
    var fraction: Double?
    var active = false
    /// Replaces the update offer with this step.
    var shown = false
    var canRetry = false

    static let idle = IrisDriveUpdateProgress()

    init() {}

    init(_ json: [String: Any]) {
        stage = json["stage"] as? String ?? "idle"
        text = json["text"] as? String ?? ""
        fraction = (json["fraction"] as? NSNumber)?.doubleValue
        active = json["active"] as? Bool ?? false
        shown = json["shown"] as? Bool ?? false
        canRetry = json["can_retry"] as? Bool ?? false
    }
}

/// The install step as the core words it; after a failure, the reason and Try Again.
struct IrisDriveUpdateProgressRow: View {
    let progress: IrisDriveUpdateProgress
    let retry: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            if progress.canRetry {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
            } else if progress.active {
                ProgressView()
                    .controlSize(.small)
            } else {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            }
            Text(progress.text)
                .font(.callout)
                .lineLimit(2)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let fraction = progress.fraction {
                ProgressView(value: fraction)
                    .frame(width: 140)
            }
            if progress.canRetry {
                Button("Try Again", action: retry)
            }
        }
    }
}
