import SwiftUI

extension View {
    func contentLinkConfirmationDialog(model: IrisDriveMobileModel) -> some View {
        confirmationDialog(
            model.pendingContentLink?.title ?? "Open file?",
            isPresented: Binding(
                get: { model.pendingContentLink != nil },
                set: { isPresented in
                    if !isPresented {
                        model.cancelPendingContentLink()
                    }
                }
            ),
            titleVisibility: .visible
        ) {
            Button("Open") {
                model.openPendingContentLink()
            }
            Button("Save to Drive") {
                model.savePendingContentLink()
            }
            Button("Cancel", role: .cancel) {
                model.cancelPendingContentLink()
            }
        }
    }
}
