import Foundation
import SwiftUI

struct IrisDriveFriendBackups {
    let backupNpub: String
    let invite: String
    let capacityBytes: UInt64
    let usedBytes: UInt64
    let friends: [IrisDriveBackupFriend]
    let error: String

    init(json: [String: Any]) {
        backupNpub = json["backup_npub"] as? String ?? ""
        invite = json["invite"] as? String ?? ""
        capacityBytes = (json["capacity_bytes"] as? NSNumber)?.uint64Value ?? 0
        usedBytes = (json["used_bytes"] as? NSNumber)?.uint64Value ?? 0
        friends = (json["friends"] as? [[String: Any]] ?? []).map(IrisDriveBackupFriend.init)
        error = json["error"] as? String ?? ""
    }
}

struct IrisDriveBackupFriend: Identifiable {
    var id: String { npub }
    let npub: String
    let label: String
    let quotaBytes: UInt64
    let usedBytes: UInt64
    let state: String
    let stateLabel: String
    let detail: String

    init(json: [String: Any]) {
        npub = json["npub"] as? String ?? ""
        label = json["label"] as? String ?? ""
        quotaBytes = (json["quota_bytes"] as? NSNumber)?.uint64Value ?? 0
        usedBytes = (json["used_bytes"] as? NSNumber)?.uint64Value ?? 0
        state = json["state"] as? String ?? ""
        stateLabel = json["state_label"] as? String ?? ""
        detail = json["detail"] as? String ?? ""
    }
}

struct FriendBackupsSection: View {
    @ObservedObject var status: IrisDriveStatus
    let controller: AppDelegate
    @State private var capacityInput = "0"
    @State private var savingCapacity = false
    @State private var showAddFriend = false
    @State private var editingFriend: IrisDriveBackupFriend?
    @State private var error = ""

    private var backups: IrisDriveFriendBackups { status.friendBackups }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Friends").font(.title3.weight(.semibold))
                Spacer()
                IrisDriveCopyButton(title: "Copy invite", systemImage: "link") {
                    irisDriveCopyToPasteboard(backups.invite)
                }
                .disabled(backups.invite.isEmpty)
                IrisDriveCopyButton(title: "Copy backup npub", systemImage: "doc.on.doc") {
                    irisDriveCopyToPasteboard(backups.backupNpub)
                }
                .disabled(backups.backupNpub.isEmpty)
            }
            Text("Back up privately with friends. You both add each other using your backup identity.")
                .font(.callout)
                .foregroundStyle(.secondary)
            HStack(spacing: 8) {
                Text("Space you share")
                TextField("0", text: $capacityInput)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 75)
                    .accessibilityLabel("Total space shared with friends in GB")
                    .onSubmit(saveCapacity)
                Text("GB").foregroundStyle(.secondary)
                Button(savingCapacity ? "Saving…" : "Save", action: saveCapacity)
                    .disabled(savingCapacity || backupBytesFromGigabytes(capacityInput) == nil)
                Text("\(backupByteLabel(backups.usedBytes)) used")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button { showAddFriend = true } label: {
                    Label("Add friend", systemImage: "person.badge.plus")
                }
            }
            if backups.friends.isEmpty {
                Text("Add a friend's invite or backup npub to get started.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 6)
            } else {
                ForEach(backups.friends) { friend in
                    friendRow(friend)
                }
            }
            HStack(spacing: 8) {
                Button {
                    controller.exportFriendBackupRecovery { result in
                        if case .failure(let failure) = result { error = failure.localizedDescription }
                        else { error = "" }
                    }
                } label: {
                    Label("Save recovery file", systemImage: "key.fill")
                }
                .disabled(backups.backupNpub.isEmpty)
                Text("Needed to recover your backup if this device is lost.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if !error.isEmpty || !backups.error.isEmpty {
                Text(error.isEmpty ? backups.error : error)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
            }
        }
        .onAppear {
            capacityInput = backupGigabytes(backups.capacityBytes)
            if status.pendingBackupInvite != nil { showAddFriend = true }
        }
        .onChange(of: backups.capacityBytes) { _, value in capacityInput = backupGigabytes(value) }
        .onChange(of: status.pendingBackupInvite) { _, invite in
            if invite != nil { showAddFriend = true }
        }
        .sheet(isPresented: $showAddFriend, onDismiss: { status.pendingBackupInvite = nil }) {
            BackupFriendEditor(controller: controller, friend: nil, initialContact: status.pendingBackupInvite ?? "")
        }
        .sheet(item: $editingFriend) { friend in
            BackupFriendEditor(controller: controller, friend: friend)
        }
    }

    private func friendRow(_ friend: IrisDriveBackupFriend) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "person.fill").foregroundStyle(.secondary).frame(width: 24)
            VStack(alignment: .leading, spacing: 3) {
                Text(friend.label.isEmpty ? shortValue(friend.npub) : friend.label)
                    .font(.callout.weight(.medium))
                Text("\(backupByteLabel(friend.usedBytes)) used · \(backupByteLabel(friend.quotaBytes)) offered")
                    .font(.caption).foregroundStyle(.secondary)
                if !friend.detail.isEmpty {
                    Text(friend.detail).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 8)
            Text(friend.stateLabel).font(.caption).foregroundStyle(.secondary)
            Button("Edit") { editingFriend = friend }
            Button(role: .destructive) {
                controller.removeBackupFriend(friend.npub) { result in
                    if case .failure(let failure) = result { error = failure.localizedDescription }
                    else { error = "" }
                }
            } label: {
                Image(systemName: "trash")
            }
            .help("Remove friend")
            .accessibilityLabel("Remove \(friend.label.isEmpty ? "friend" : friend.label)")
        }
        .padding(.vertical, 6)
    }

    private func saveCapacity() {
        guard let bytes = backupBytesFromGigabytes(capacityInput), !savingCapacity else { return }
        savingCapacity = true
        error = ""
        controller.setFriendBackupCapacity(bytes) { result in
            savingCapacity = false
            if case .failure(let failure) = result { error = failure.localizedDescription }
        }
    }
}

private struct BackupFriendEditor: View {
    let controller: AppDelegate
    let friend: IrisDriveBackupFriend?
    var initialContact = ""
    @Environment(\.dismiss) private var dismiss
    @State private var contact = ""
    @State private var label = ""
    @State private var quotaInput = "0"
    @State private var saving = false
    @State private var error = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(friend == nil ? "Add backup friend" : "Edit backup friend").font(.title2.weight(.semibold))
            VStack(alignment: .leading, spacing: 5) {
                Text("Friend's invite or backup npub").font(.callout)
                TextField("iris-drive://backup?npub=…", text: $contact)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                    .disabled(friend != nil)
            }
            TextField("Name (optional, only visible to you)", text: $label).textFieldStyle(.roundedBorder)
            HStack {
                Text("Space you offer")
                TextField("0", text: $quotaInput).textFieldStyle(.roundedBorder).frame(width: 90)
                    .accessibilityLabel("Space offered to this friend in GB")
                Text("GB").foregroundStyle(.secondary)
            }
            Text("Choose the space yourself. 0 GB requests backup space without offering space on this device.")
                .font(.callout).foregroundStyle(.secondary)
            Text("Share your own invite too. Backup identities are separate from social npubs.")
                .font(.callout).foregroundStyle(.secondary)
            if !error.isEmpty {
                Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(saving ? "Saving…" : "Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(saving || contact.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                              || backupBytesFromGigabytes(quotaInput) == nil)
            }
        }
        .padding(24)
        .frame(width: 430)
        .onAppear {
            contact = initialContact
            if let friend {
                contact = friend.npub
                label = friend.label
                quotaInput = backupGigabytes(friend.quotaBytes)
            }
        }
    }

    private func save() {
        guard let bytes = backupBytesFromGigabytes(quotaInput) else { return }
        saving = true
        error = ""
        controller.addBackupFriend(contact: contact, label: label, bytes: bytes) { result in
            saving = false
            switch result {
            case .success: dismiss()
            case .failure(let failure): error = failure.localizedDescription
            }
        }
    }
}

private func backupByteLabel(_ bytes: UInt64) -> String {
    if bytes == 0 { return "0 B" }
    return ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .decimal)
}

private func backupGigabytes(_ bytes: UInt64) -> String {
    NSDecimalNumber(decimal: Decimal(bytes) / 1_000_000_000).stringValue
}

private func backupBytesFromGigabytes(_ text: String) -> UInt64? {
    let input = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !input.isEmpty, input.filter({ $0 == "." }).count <= 1,
          input.utf8.allSatisfy({ (48...57).contains($0) || $0 == 46 }),
          let gigabytes = Decimal(string: input, locale: Locale(identifier: "en_US_POSIX")) else { return nil }
    var bytes = gigabytes * 1_000_000_000
    var rounded = Decimal()
    NSDecimalRound(&rounded, &bytes, 0, .plain)
    guard bytes >= 0, bytes <= Decimal(UInt64.max), bytes == rounded else { return nil }
    return NSDecimalNumber(decimal: bytes).uint64Value
}
