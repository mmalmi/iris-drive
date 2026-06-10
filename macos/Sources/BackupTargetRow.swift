import SwiftUI

struct BackupTargetRow: View {
    let target: IrisDriveBackupTarget
    let onCheck: (@escaping () -> Void) -> Void
    let onRemove: () -> Void
    @State private var expanded = false
    @State private var checking = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                Button {
                    withAnimation(.easeInOut(duration: 0.15)) {
                        expanded.toggle()
                    }
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: target.iconName)
                            .frame(width: 24)
                            .foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(target.title)
                                .font(.callout.weight(.medium))
                                .lineLimit(1)
                            Text(target.detail)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                Spacer()
                Text(statusLine)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Button {
                    guard !checking else { return }
                    checking = true
                    onCheck {
                        checking = false
                    }
                } label: {
                    if checking {
                        Text("Checking 0 of 1")
                    } else {
                        Label("Check", systemImage: "checkmark.shield")
                    }
                }
                .disabled(checking)
                .help("Check \(target.target)")
                Button(role: .destructive) {
                    onRemove()
                } label: {
                    Label(removeLabel, systemImage: "trash")
                }
                .help("Remove \(target.target)")
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(expanded ? 90 : 0))
            }

            if expanded {
                VStack(alignment: .leading, spacing: 8) {
                    DetailRow(label: "Destination", value: target.target, copyable: true)
                    if let uploaded = target.uploaded, let total = target.totalHashes {
                        DetailRow(label: "Progress", value: "\(uploaded)/\(total)")
                    }
                    if let checkState = target.checkState {
                        DetailRow(label: "Check", value: checkState)
                    }
                    if let error = target.error {
                        DetailRow(label: "Error", value: error)
                    }
                    if let checkedAt = target.checkedAt {
                        DetailRow(label: "Last checked", value: checkedAgeLine(checkedAt))
                    }
                    if let latencyMs = target.latencyMs {
                        DetailRow(label: "Latency", value: "\(latencyMs) ms")
                    }
                    if let bandwidth = target.downloadBytesPerSecond {
                        DetailRow(label: "Bandwidth", value: "\(byteString(bandwidth))/s")
                    }
                    if let sampled = target.sampledHashes {
                        DetailRow(
                            label: "Sample",
                            value: "\(sampled) checked, \(target.missing ?? 0) missing, \(target.unknown ?? 0) unknown"
                        )
                    }
                }
                .padding(.top, 12)
            }
        }
        .padding(12)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var statusLine: String {
        if checking {
            return "Checking 0 of 1"
        }
        if let checkedAt = target.checkedAt {
            let checked = checkedAgeLine(checkedAt)
            if let checkState = target.checkState,
               !["ok", "verified"].contains(checkState.lowercased()) {
                return "Check \(checkState) | \(checked)"
            }
            return checked
        }
        if let uploaded = target.uploaded,
           let total = target.totalHashes,
           total > 0,
           uploaded < total {
            return "Syncing \(Int(Double(uploaded) / Double(total) * 100))%"
        }
        switch target.state.lowercased() {
        case "synced":
            if target.checkState?.lowercased() == "verified" {
                return "Up to date | verified"
            }
            return "Up to date"
        default:
            return target.state
        }
    }

    private var removeLabel: String {
        target.kind == "blossom" ? "Remove file server" : "Remove target"
    }

    private func checkedAgeLine(_ checkedAt: Int) -> String {
        let age = max(0, Int(Date().timeIntervalSince1970) - checkedAt)
        if age < 60 {
            return "Checked just now"
        }
        let minutes = age / 60
        if minutes < 60 {
            return "Checked \(minutes) \(minutes == 1 ? "minute" : "minutes") ago"
        }
        let hours = minutes / 60
        if hours < 48 {
            return "Checked \(hours) \(hours == 1 ? "hour" : "hours") ago"
        }
        let days = hours / 24
        return "Checked \(days) \(days == 1 ? "day" : "days") ago"
    }

    private func byteString(_ bytes: Int) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }
}
