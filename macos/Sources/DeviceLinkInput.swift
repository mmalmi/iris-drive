import Foundation

extension IrisDriveControlPanel {
    func submitSetupOwner(_ value: String, force _: Bool, inputIsComplete: Bool = false) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard inputIsComplete else { return }
        guard submittedSetupOwner != trimmed else { return }
        submittedSetupOwner = trimmed
        controller.linkDevice(owner: trimmed)
    }
}
