import Foundation

extension IrisDriveControlPanel {
    func submitSetupOwner(_ value: String, force: Bool) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              force,
              submittedSetupOwner != trimmed else { return }
        submittedSetupOwner = trimmed
        controller.linkDevice(owner: trimmed)
    }
}
