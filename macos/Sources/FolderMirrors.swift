import AppKit
import SwiftUI

struct MirroredFolder: Decodable, Identifiable {
    struct Report: Decodable {
        let finished_at: TimeInterval
        let copied, unchanged, pending: Int
        let issues: [String]
    }
    let id, source, destination: String
    let bookmark: String?
    let due, running: Bool
    let last_run: Report?
}

final class FolderMirrors: ObservableObject {
    @Published var folders: [MirroredFolder] = []
    @Published var error: String?
    @Published var active: Set<String> = []
    @Published var loaded = false
    private let directory: String
    private let queue = DispatchQueue(label: "to.iris.drive.mirror-runs", qos: .utility)
    private var timer: Timer?
    private var reading = false

    init(directory: String) { self.directory = directory }

    func start() {
        timer = Timer.scheduledTimer(withTimeInterval: 15, repeats: true) { [weak self] _ in
            guard IrisDriveStatus.shared.setupComplete, IrisDriveStatus.shared.daemonRunning else { return }
            self?.refresh(schedule: true)
        }
    }

    func refresh(schedule: Bool = false) {
        if IrisDriveScreenshotFixtures.enabled {
            folders = IrisDriveScreenshotFixtures.mirroredFolders
            loaded = true
            return
        }
        guard !reading else { return }
        reading = true
        let directory = directory
        DispatchQueue.global(qos: .utility).async {
            let result = Result { () throws -> [MirroredFolder] in
                let value = try IrisDriveDesktopCore.mirror(["action": "list"], in: directory)
                let data = try JSONSerialization.data(withJSONObject: value["folders"] ?? NSNull())
                return try JSONDecoder().decode([MirroredFolder].self, from: data)
            }
            DispatchQueue.main.async {
                self.reading = false
                self.loaded = true
                switch result {
                case .success(let folders):
                    self.folders = folders
                    if schedule {
                        for folder in folders where folder.bookmark != nil && folder.due && !folder.running {
                            self.run(folder, scheduled: true)
                        }
                    }
                case .failure(let error): self.error = error.localizedDescription
                }
            }
        }
    }

    func add(source: URL, destination: String, completion: @escaping (Bool) -> Void) {
        error = nil
        let directory = directory
        queue.async {
            let scoped = source.startAccessingSecurityScopedResource()
            defer { if scoped { source.stopAccessingSecurityScopedResource() } }
            let result = Result {
                let data = try source.bookmarkData(options: [.withSecurityScope, .securityScopeAllowOnlyReadAccess],
                                                   includingResourceValuesForKeys: nil, relativeTo: nil)
                _ = try IrisDriveDesktopCore.mirror(["action": "add", "source": source.path, "destination": destination,
                                                     "bookmark": data.base64EncodedString()], in: directory)
            }
            DispatchQueue.main.async {
                do {
                    try result.get()
                    completion(true)
                    self.refresh(schedule: true)
                } catch { self.error = error.localizedDescription; completion(false) }
            }
        }
    }

    func run(_ folder: MirroredFolder, scheduled: Bool = false) {
        guard !active.contains(folder.id) else { return }
        active.insert(folder.id)
        if !scheduled { error = nil }
        let directory = directory
        queue.async {
            var request: [String: Any] = ["action": "run", "id": folder.id, "destination": folder.destination, "scheduled": scheduled]
            var access: URL?
            if let bookmark = folder.bookmark {
                do {
                    guard let data = Data(base64Encoded: bookmark) else { throw CocoaError(.fileReadCorruptFile) }
                    var stale = false
                    let url = try URL(resolvingBookmarkData: data, options: [.withSecurityScope, .withoutUI],
                                      relativeTo: nil, bookmarkDataIsStale: &stale)
                    let scoped = url.startAccessingSecurityScopedResource()
                    if scoped { access = url }
                    guard (scoped || !currentProcessHasEntitlement("com.apple.security.app-sandbox")),
                          url.resolvingSymlinksInPath().standardizedFileURL.path == folder.source else {
                        throw CocoaError(.fileReadNoPermission)
                    }
                } catch {
                    request["access_error"] = "Folder access is unavailable. Reconnect the original folder or add it again. " + error.localizedDescription
                }
            }
            defer { access?.stopAccessingSecurityScopedResource() }
            let result = Result { try IrisDriveDesktopCore.mirror(request, in: directory) }
            DispatchQueue.main.async {
                self.active.remove(folder.id)
                if case .failure(let error) = result { self.error = error.localizedDescription }
                self.refresh()
            }
        }
    }

    func remove(_ folder: MirroredFolder) {
        error = nil
        let directory = directory
        queue.async {
            let result = Result { try IrisDriveDesktopCore.mirror(["action": "remove", "destination": folder.destination], in: directory) }
            DispatchQueue.main.async {
                if case .failure(let error) = result { self.error = error.localizedDescription }
                self.refresh()
            }
        }
    }
}
