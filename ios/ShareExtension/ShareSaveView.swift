import SwiftUI
import UniformTypeIdentifiers

/// Something handed to the share sheet, staged on disk at `source` and ready
/// to be saved under `name`.
struct PendingSharedItem: Identifiable, Equatable {
    let id = UUID()
    var name: String
    let contentType: UTType
    let source: URL
}

/// What the "Save to Iris Drive" sheet shows: the items, where they go, and
/// how saving went. Nothing is written until the person taps Save.
@MainActor
final class ShareSaveModel: ObservableObject {
    enum Phase: Equatable {
        case loading
        case notSetUp
        case ready
        case saving
        case syncing
        case saved(String)
        case failed(String)
    }

    @Published var phase = Phase.loading
    @Published var items: [PendingSharedItem] = []
    @Published var folders: [String] = []
    /// Folder path inside My Drive; empty is My Drive itself.
    @Published var folder = ""

    var canSave: Bool {
        phase == .ready && !items.isEmpty
            && items.allSatisfy { !$0.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    var destinationName: String {
        folder.isEmpty ? "My Drive" : (folder.split(separator: "/").last.map(String.init) ?? folder)
    }

    /// `folders` is nil when Iris Drive has no profile on this device.
    func loaded(_ loaded: [PendingSharedItem], folders: [String]?) {
        guard let folders else {
            phase = .notSetUp
            return
        }
        self.folders = folders
        items = loaded
        phase = loaded.isEmpty ? .failed("Nothing here can be saved to Iris Drive.") : .ready
    }

    func save(using saveItem: @escaping (PendingSharedItem, String) throws -> Void,
              sync: (() -> String?)? = nil, done: @escaping () -> Void) {
        guard canSave else { return }
        phase = .saving
        let items = items
        let folder = folder
        let destination = destinationName
        Task.detached {
            do {
                for item in items {
                    try saveItem(item, folder)
                }
                if let sync {
                    await MainActor.run { self.phase = .syncing }
                    if let pending = sync() {
                        await MainActor.run { self.phase = .saved(pending) }
                        return
                    }
                }
                await MainActor.run {
                    self.phase = .saved(items.count == 1 ? "Saved to \(destination)" : "Saved \(items.count) items to \(destination)")
                }
                try? await Task.sleep(nanoseconds: 900_000_000)
                await MainActor.run { done() }
            } catch {
                await MainActor.run { self.phase = .failed(error.localizedDescription) }
            }
        }
    }
}

struct ShareSaveView: View {
    @ObservedObject var model: ShareSaveModel
    let onCancel: () -> Void
    let onSave: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                switch model.phase {
                case .loading:
                    ProgressView("Getting ready…")
                case .notSetUp:
                    Text("Open Iris Drive and set it up on this device first.")
                        .accessibilityIdentifier("shareSaveNotSetUp")
                case .ready, .saving:
                    Section(model.items.count == 1 ? "Name" : "\(model.items.count) items") {
                        if model.items.count == 1 {
                            TextField("Name", text: $model.items[0].name)
                                .autocorrectionDisabled()
                                .accessibilityIdentifier("shareSaveName")
                        } else {
                            ForEach(model.items) { item in
                                Text(item.name).lineLimit(1)
                            }
                        }
                    }
                    Section {
                        Picker("Save in", selection: $model.folder) {
                            Text("My Drive").tag("")
                            ForEach(model.folders, id: \.self) { folder in
                                Text(folder).tag(folder)
                            }
                        }
                        .pickerStyle(.navigationLink)
                        .accessibilityIdentifier("shareSaveFolder")
                    }
                    if model.phase == .saving {
                        ProgressView("Saving…")
                    }
                case .syncing:
                    ProgressView("Syncing…")
                case .saved(let message):
                    Label(message, systemImage: "checkmark.circle.fill")
                        .accessibilityIdentifier("shareSaveResult")
                case .failed(let message):
                    Label(message, systemImage: "exclamationmark.triangle.fill")
                        .accessibilityIdentifier("shareSaveError")
                }
            }
            .navigationTitle("Iris Drive")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(closesOnly ? "Close" : "Cancel", action: onCancel)
                        .accessibilityIdentifier("shareSaveCancel")
                        .disabled(model.phase == .saving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    if !closesOnly {
                        Button("Save", action: onSave)
                            .disabled(!model.canSave)
                            .accessibilityIdentifier("shareSaveButton")
                    }
                }
            }
        }
    }

    /// Nothing left to decide: only closing the sheet makes sense.
    private var closesOnly: Bool {
        switch model.phase {
        case .notSetUp, .failed, .saved, .syncing: true
        default: false
        }
    }
}
