import SwiftUI
import UIKit

extension IrisDriveMobileModel {
    /// Folders this phone asked to join and is waiting to be let into.
    var pendingShareJoins: [NativePendingShareJoin] {
        lastState?.ui.pendingShareJoins ?? []
    }

    /// The folder's link for `role`, made on first use.
    func shareLink(shareId: String, role: String) -> String {
        dispatch(["type": "create_share_join_link", "share_id": shareId, "role": role])
        return lastShareInvite
    }

    /// My Drive folders a share can start from.
    var shareableFolders: [String] {
        lastState?.ui.shareableFolders ?? []
    }

    /// Take a shared folder out of My Drive on this phone; access stays.
    func removeShareFromMyDrive(shareId: String) {
        dispatch(["type": "remove_share_shortcuts", "share_id": shareId])
    }

    func cancelShareJoin(shareId: String) {
        dispatch(["type": "cancel_share_join", "share_id": shareId])
    }

    func disableShareLinks(shareId: String) {
        dispatch(["type": "disable_share_join_links", "share_id": shareId])
        statusTitle = "Links turned off"
        statusDetail = "People already in keep access."
    }

    /// drive.iris.to join and invite links open Shares with the link ready to join.
    func handleShareLink(_ linkInput: NativeLinkInputClassification, url: URL) -> Bool {
        guard linkInput.kind == "share_join" || linkInput.kind == "share_invite" else { return false }
        shareInviteInput = url.absoluteString
        shareDialogRequestId &+= 1
        let name = linkInput.shareDisplayName.isEmpty ? "this shared folder" : "“\(linkInput.shareDisplayName)”"
        statusTitle = linkInput.isValid ? "Join \(name)?" : "This link is not valid"
        statusDetail = linkInput.isValid ? "Tap Join to ask for access." : linkInput.error
        return true
    }
}

/// Share a folder by link. Anyone who opens it can ask to join; this phone
/// lets them in when Iris Drive syncs. No app is needed to open the link.
struct ShareLinkSheet: View {
    @ObservedObject var model: IrisDriveMobileModel
    let share: IrisDriveShare
    @Environment(\.dismiss) private var dismiss
    @State private var role = "reader"
    @State private var link = ""

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("People with the link can", selection: $role) {
                        Text("View").tag("reader")
                        Text("Edit").tag("editor")
                    }
                    .pickerStyle(.segmented)
                    .accessibilityIdentifier("shareLinkRole")
                } footer: {
                    Text("Anyone with this link can join. Iris Drive lets them in when it syncs. They don't need the app — the link opens in a browser.")
                }
                Section {
                    if let url = URL(string: link), !link.isEmpty {
                        ShareLink(item: url) {
                            Label("Send link", systemImage: "square.and.arrow.up")
                        }
                        .accessibilityIdentifier("shareLinkSend")
                        Button {
                            model.copyToClipboard(link, feedback: "Link copied")
                        } label: {
                            Label("Copy link", systemImage: "link")
                        }
                        .accessibilityIdentifier("shareLinkCopy")
                    } else {
                        ProgressView()
                    }
                }
                if share.members.count > 1 {
                    Section("People") {
                        ForEach(share.members.filter { $0.status != "revoked" }) { member in
                            LabeledContent(
                                member.profileId == model.localProfileId ? "You" : (member.displayName.isEmpty ? "Someone" : member.displayName),
                                value: member.roleLabel.isEmpty ? member.role : member.roleLabel
                            )
                        }
                    }
                }
                Section {
                    Button("Turn off links", role: .destructive) {
                        model.disableShareLinks(shareId: share.shareId)
                        dismiss()
                    }
                }
            }
            .navigationTitle("Share “\(share.displayName.isEmpty ? "Shared folder" : share.displayName)”")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .onAppear { link = model.shareLink(shareId: share.shareId, role: role) }
            .onChange(of: role) { _, newRole in
                link = model.shareLink(shareId: share.shareId, role: newRole)
            }
        }
    }
}

struct PendingShareJoinRows: View {
    let joins: [NativePendingShareJoin]
    let onCancel: (NativePendingShareJoin) -> Void

    var body: some View {
        ForEach(joins) { join in
            HStack(spacing: 12) {
                ProgressView()
                VStack(alignment: .leading, spacing: 2) {
                    Text("Waiting to join “\(join.displayName)”")
                    Text("You get access when someone who manages it has Iris Drive open.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .accessibilityIdentifier("pendingShareJoin")
            .swipeActions {
                Button("Cancel", role: .destructive) { onCancel(join) }
            }
        }
    }
}
