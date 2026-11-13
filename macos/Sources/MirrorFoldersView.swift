import AppKit
import SwiftUI

struct MirrorFoldersView: View {
    @ObservedObject var model: FolderMirrors
    @State private var source: URL?
    @State private var destination = ""
    @State private var configuring = false
    @State private var saving = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Mirrored folders").font(.title2.weight(.semibold))
                Spacer()
                Button { chooseSource() } label: { Label("Mirror folder…", systemImage: "plus") }
                    .buttonStyle(.borderedProminent)
            }
            Text("Copy new and changed files into My Drive every 5 minutes. Files removed from the source stay in Iris Drive.")
                .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let error = model.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if !model.loaded { ProgressView().controlSize(.small) }
            else if model.folders.isEmpty {
                ContentUnavailableView("No mirrored folders", systemImage: "folder")
                    .frame(maxWidth: .infinity, minHeight: 140)
            }
            ForEach(model.folders) { folder in row(folder) }
        }
        .onAppear { model.refresh() }
        .sheet(isPresented: $configuring) {
            VStack(alignment: .leading, spacing: 16) {
                Text("Mirror folder").font(.title3.weight(.semibold))
                if let source {
                    LabeledContent("Source") { Text(source.path).truncationMode(.middle).lineLimit(1) }
                    TextField("Destination in My Drive", text: $destination, prompt: Text("Folder name"))
                        .textFieldStyle(.roundedBorder)
                    Text("Files will appear in My Drive/\(destination).").font(.callout).foregroundStyle(.secondary)
                    if let error = model.error { Text(error).foregroundStyle(.red) }
                    HStack {
                        Spacer()
                        Button("Cancel") { configuring = false }.keyboardShortcut(.cancelAction)
                        Button("Start mirroring") {
                            saving = true
                            model.add(source: source, destination: destination) { added in
                                saving = false
                                if added { configuring = false }
                            }
                        }.keyboardShortcut(.defaultAction).disabled(saving || destination.trimmingCharacters(in: .whitespaces).isEmpty)
                    }.disabled(saving)
                }
            }.padding(24).frame(width: 440)
        }
    }

    private func chooseSource() {
        let panel = NSOpenPanel()
        panel.title = "Choose a folder to mirror"
        panel.prompt = "Choose folder"
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let selected = panel.url else { return }
        model.error = nil
        source = selected
        destination = selected.lastPathComponent
        configuring = true
    }

    private func row(_ folder: MirroredFolder) -> some View {
        let running = folder.running || model.active.contains(folder.id)
        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("My Drive/\(folder.destination)", systemImage: "folder.fill").font(.headline)
                Spacer()
                Button("Run now") { model.run(folder) }.disabled(running)
                Menu {
                    Button("Stop mirroring", role: .destructive) { model.remove(folder) }
                } label: { Image(systemName: "ellipsis.circle") }
                    .menuStyle(.borderlessButton).fixedSize().disabled(running).accessibilityLabel("Mirror options")
            }
            Text(folder.source).lineLimit(1).truncationMode(.middle).foregroundStyle(.secondary).help(folder.source)
            if running { HStack { ProgressView().controlSize(.small); Text("Checking files…") } }
            if let report = folder.last_run {
                HStack(spacing: 4) {
                    Text("Last checked")
                    Text(Date(timeIntervalSince1970: report.finished_at), style: .relative)
                    Text("ago · \(report.copied) copied · \(report.unchanged) unchanged")
                }.font(.caption).foregroundStyle(.secondary)
                if report.pending > 0 {
                    DisclosureGroup(report.pending == 1 ? "1 item needs attention" : "\(report.pending) items need attention") {
                        ForEach(Array(report.issues.enumerated()), id: \.offset) { entry in
                            Text(entry.element).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
                        }
                    }
                }
            } else if !running {
                Text("Waiting for first check").font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(14).frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
    }
}
