import AppKit
import SwiftUI

/// A folder this Mac asked to join from a link, still waiting to be let in.
struct IrisDrivePendingShareJoin: Identifiable, Equatable {
    let id: String
    let displayName: String
    let roleLabel: String

    init(json: [String: Any]) {
        id = json["share_id"] as? String ?? UUID().uuidString
        displayName = json["display_name"] as? String ?? "Shared folder"
        roleLabel = json["role_label"] as? String ?? ""
    }
}

/// A join link opened from a browser or another app, waiting for the user to confirm.
struct IrisDriveJoinLinkRequest: Identifiable, Equatable {
    let id = UUID()
    let link: String
    let displayName: String
}

extension AppDelegate {
    /// Make (or reuse) the folder's link for `role` and copy it.
    func createShareLink(shareId: String, role: String, completion: @escaping (String?) -> Void) {
        if screenshotFixtureMode {
            return completion("https://drive.iris.to/#/share-join/demo-\(role)")
        }
        dispatchNativeAction(
            ["type": "create_share_join_link", "share_id": shareId, "role": role],
            progress: "Making link",
            success: "Link ready"
        ) {
            completion(IrisDriveStatus.shared.lastShareInviteURL)
        } failure: { _ in
            completion(nil)
        }
    }

    func disableShareLinks(shareId: String) {
        dispatchNativeAction(
            ["type": "disable_share_join_links", "share_id": shareId],
            progress: "Turning off links",
            success: "Links turned off. People already in keep access."
        )
    }

    /// drive.iris.to join and invite links open the app's Join prompt.
    func handleShareLinkURL(_ url: URL, classification: [String: Any]) -> Bool {
        let kind = classification["kind"] as? String ?? ""
        guard kind == "share_join" || kind == "share_invite" else { return false }
        showControlPanel()
        guard classification["is_valid"] as? Bool ?? false else {
            updateStatus(classification["error"] as? String ?? "This link is not valid")
            return true
        }
        let name = (classification["share_display_name"] as? String ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        IrisDriveStatus.shared.pendingJoinLink = IrisDriveJoinLinkRequest(
            link: url.absoluteString,
            displayName: name.isEmpty ? "this shared folder" : name
        )
        return true
    }

    /// Open in the default browser itself, so a drive.iris.to link is not
    /// handed straight back to this app as a universal link.
    func openInWebBrowser(_ url: URL) {
        guard let browser = NSWorkspace.shared.urlForApplication(toOpen: URL(string: "http://example.com")!) else {
            updateStatus("No web browser found")
            return
        }
        NSWorkspace.shared.open([url], withApplicationAt: browser, configuration: NSWorkspace.OpenConfiguration())
    }

    func joinSharedFolder(_ link: String) {
        let link = link.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !link.isEmpty else {
            updateStatus("Paste a link to join")
            return
        }
        let isJoinLink = IrisDriveDesktopCore.classifyLinkInput(link)["kind"] as? String == "share_join"
        dispatchNativeAction(
            ["type": "accept_share_invite", "invite": link],
            progress: isJoinLink ? "Asking to join" : "Joining",
            success: isJoinLink ? "Asked to join. It appears here once you're let in." : "Joined"
        )
    }
}

/// Share a folder by link: anyone who opens it can ask to join, and this
/// Mac lets them in. People without the app use it on drive.iris.to.
struct InviteShareMemberSheet: View {
    let controller: AppDelegate
    @ObservedObject var status: IrisDriveStatus
    let share: IrisDriveShareStatus
    @Environment(\.dismiss) private var dismiss
    @State private var role = "reader"
    @State private var link = ""
    @State private var loading = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Share “\(shareSheetName)”")
                .font(.title3.weight(.semibold))
            Picker("People with the link can", selection: $role) {
                Text("View").tag("reader")
                Text("Edit").tag("editor")
            }
            .pickerStyle(.segmented)
            Group {
                if link.isEmpty {
                    ProgressView()
                        .opacity(loading ? 1 : 0)
                } else {
                    IrisDriveQRCodeView(value: link)
                }
            }
            .frame(width: 180, height: 180)
            .frame(maxWidth: .infinity, alignment: .center)
            Text("Anyone with this link can join. Your Mac lets them in while Iris Drive is running. They don't need the app — the link opens in a browser.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if share.members.count > 1 {
                Text(shareSheetPeople)
                    .font(.callout)
            }
            HStack {
                Button(role: .destructive) {
                    controller.disableShareLinks(shareId: share.shareId)
                    dismiss()
                } label: {
                    Text("Turn off links")
                }
                Spacer()
                IrisDriveCopyButton(title: "Copy link", systemImage: "link") {
                    irisDriveCopyToPasteboard(link, feedback: "Link copied")
                }
                .disabled(link.isEmpty)
                Button("Done") { dismiss() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 420)
        .onAppear(perform: refreshLink)
        .onChange(of: role) { _, _ in refreshLink() }
    }

    private var shareSheetName: String {
        let name = share.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? "Shared folder" : name
    }

    private var shareSheetPeople: String {
        let names = share.members
            .filter { $0.status != "revoked" && $0.profileId != status.profileId }
            .map { $0.displayName.isEmpty ? "Someone" : $0.displayName }
        return names.isEmpty ? "" : "Shared with " + names.joined(separator: ", ")
    }

    private func refreshLink() {
        link = ""
        loading = true
        let requested = role
        controller.createShareLink(shareId: share.shareId, role: requested) { url in
            guard requested == role else { return }
            loading = false
            link = url ?? ""
        }
    }
}

/// Folders this Mac asked to join and is waiting to be let into.
struct PendingShareJoinRows: View {
    let joins: [IrisDrivePendingShareJoin]

    var body: some View {
        ForEach(joins) { join in
            HStack(spacing: 10) {
                ProgressView().controlSize(.small)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Waiting to join “\(join.displayName)”")
                    Text("You get access when someone who manages it has Iris Drive open.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 8))
        }
    }
}
