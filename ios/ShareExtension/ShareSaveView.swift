import FileProvider
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
        case waitingForFolders
        case notSetUp
        case ready
        case saving
        case waitingToSave
        case syncing
        case saved(String)
        case failed(String)
    }

    @Published var phase = Phase.loading
    @Published var items: [PendingSharedItem] = []
    @Published var folders: [String] = []
    /// Folder path inside My Drive; empty is My Drive itself.
    @Published var folder = ""

    private(set) var savedItemCount = 0

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

    func prepareDestinations(
        for staged: [PendingSharedItem],
        delays: [Duration] = [.milliseconds(250), .milliseconds(750), .seconds(2), .seconds(4)],
        read: @escaping @Sendable () throws -> [String]?
    ) async {
        items = staged
        phase = .loading
        var waits = delays.makeIterator()
        while !Task.isCancelled {
            do {
                let folders = try await Task.detached(priority: .userInitiated, operation: read).value
                guard !Task.isCancelled else { return }
                loaded(staged, folders: folders)
                return
            } catch {
                guard !Task.isCancelled else { return }
                let issue = error as NSError
                guard Self.hasMissingBlock(issue) else {
                    phase = .failed(issue.localizedDescription)
                    return
                }
                guard let delay = waits.next() else {
                    phase = .waitingForFolders
                    return
                }
                do { try await Task.sleep(for: delay) } catch { return }
            }
        }
    }

    nonisolated private static func hasMissingBlock(_ issue: NSError) -> Bool {
        issue.domain == NSFileProviderErrorDomain && issue.code == NSFileProviderError.serverUnreachable.rawValue
            && issue.localizedDescription.range(of: #"Missing chunk: [0-9a-f]{64}\b"#, options: .regularExpression) != nil
    }

    func save(using saveItem: @escaping (PendingSharedItem, String) throws -> Void,
              delays: [Duration] = [.milliseconds(250), .milliseconds(750), .seconds(2), .seconds(4)],
              sync: (() -> String?)? = nil, done: @escaping () -> Void) {
        guard canSave || phase == .waitingToSave else { return }
        phase = .saving
        let items = items
        let folder = folder
        let destination = destinationName
        let firstPending = savedItemCount
        Task.detached {
            do {
                var waits = delays.makeIterator()
                var index = firstPending
                while index < items.count {
                    do {
                        try saveItem(items[index], folder)
                        index += 1
                    } catch {
                        let issue = error as NSError
                        // The core opens the provider root before choosing a name or writing bytes.
                        // Failures during or after mutation must never replay an import.
                        guard issue.localizedDescription.hasPrefix("building provider root:"),
                              Self.hasMissingBlock(issue) else { throw error }
                        guard let delay = waits.next() else {
                            let nextItem = index
                            await MainActor.run {
                                self.savedItemCount = nextItem
                                self.phase = .waitingToSave
                            }
                            return
                        }
                        try await Task.sleep(for: delay)
                    }
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
    let onRetryFolders: () -> Void
    let onSave: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                switch model.phase {
                case .loading:
                    ProgressView("Getting ready…")
                case .waitingForFolders:
                    Text("Drive folders haven’t finished syncing.")
                    Button("Try Again", action: onRetryFolders)
                        .accessibilityIdentifier("shareRetryFolders")
                case .waitingToSave:
                    Text(model.savedItemCount == 0
                         ? "Waiting for the destination folder to finish syncing."
                         : "Saved \(model.savedItemCount) of \(model.items.count) items. Waiting for the destination folder to finish syncing.")
                    Button("Try Again", action: onSave)
                        .accessibilityIdentifier("shareRetrySave")
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
                    .disabled(model.phase == .saving)
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
                    .disabled(model.phase == .saving)
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
                    if !closesOnly && model.phase != .waitingToSave {
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
