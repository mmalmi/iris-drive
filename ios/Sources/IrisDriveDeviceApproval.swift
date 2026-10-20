import SwiftUI

extension IrisDriveMobileModel {
    func requestDeviceApprovalConfirmation(_ request: String) {
        let request = request.trimmingCharacters(in: .whitespacesAndNewlines)
        guard canAdminProfile, IrisDriveNativeLinkInput.isCompleteDeviceApproval(request) else {
            return
        }
        pendingDeviceApprovalRequest = request
    }

    func cancelDeviceApprovalConfirmation() {
        pendingDeviceApprovalRequest = nil
    }

    func approvePendingDevice() {
        guard let request = pendingDeviceApprovalRequest else { return }
        pendingDeviceApprovalRequest = nil
        approveDevice(request: request, label: "")
    }
}

extension View {
    func deviceApprovalConfirmationDialog(model: IrisDriveMobileModel) -> some View {
        alert(
            "Approve this device?",
            isPresented: Binding(
                get: { model.pendingDeviceApprovalRequest != nil },
                set: { if !$0 { model.cancelDeviceApprovalConfirmation() } }
            )
        ) {
            Button("Cancel", role: .cancel) {
                model.cancelDeviceApprovalConfirmation()
            }
            Button("Approve") {
                model.approvePendingDevice()
            }
        } message: {
            Text("This will add the joining device to Iris Drive.")
        }
    }
}
