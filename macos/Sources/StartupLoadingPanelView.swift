import SwiftUI

struct StartupLoadingPanelView: View {
    let showLabel: Bool

    var body: some View {
        VStack(spacing: 14) {
            Spacer()
            Image("BrandIcon")
                .resizable()
                .interpolation(.high)
                .frame(width: 88, height: 88)
            Text("Iris Drive")
                .font(.largeTitle.bold())
            if showLabel {
                ProgressView()
                    .controlSize(.small)
                    .padding(.top, 4)
                Text("Loading")
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(32)
        .frame(minWidth: 520, minHeight: 420)
    }
}
