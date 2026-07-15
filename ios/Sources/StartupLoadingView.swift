import SwiftUI

struct StartupLoadingView: View {
    let showLabel: Bool

    var body: some View {
        VStack(spacing: 14) {
            Spacer()
            Image("BrandIcon")
                .resizable()
                .interpolation(.high)
                .frame(width: 96, height: 96)
            Text("Iris Drive")
                .font(.title.bold())
            if showLabel {
                ProgressView()
                    .padding(.top, 4)
                Text("Loading")
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("startupLoadingLabel")
            }
            Spacer()
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityIdentifier("startupLoadingView")
    }
}
