import PhotosUI
import SwiftUI
import UIKit
import WebKit

private let recoveryPhraseWordCount = 12
private let irisWebBrowserExpandedFooterHeight: CGFloat = 58
private let irisWebBrowserCollapsedFooterHeight: CGFloat = 51
private let irisWebBrowserFooterClearance: CGFloat = 4

private enum MainTab: Hashable {
    case drive
    case devices
    case shares
    case backup
    case settings
}

struct IrisDriveRootView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var selectedTab = MainTab.drive
    @State private var showStartupLoading = false

    var body: some View {
        ZStack(alignment: .bottom) {
            content
            if !model.copyFeedback.isEmpty {
                CopyFeedbackToast(message: model.copyFeedback)
                    .padding(.bottom, 18)
                    .transition(.opacity.combined(with: .move(edge: .bottom)))
            }
        }
        .animation(.easeInOut(duration: 0.18), value: model.copyFeedback)
        .animation(.easeInOut(duration: 0.18), value: model.stateLoaded)
        .animation(.easeInOut(duration: 0.18), value: showStartupLoading)
        .contentLinkConfirmationDialog(model: model)
        .fullScreenCover(item: $model.webRoute) { route in
            IrisWebBrowserView(model: model, route: route)
        }
        .task(id: model.stateLoaded) {
            await revealStartupLoadingIfNeeded()
        }
    }

    @ViewBuilder
    private var content: some View {
        if !model.stateLoaded {
            StartupLoadingView(showLabel: showStartupLoading)
        } else if !model.isSetupComplete {
            if model.isRevoked {
                RevokedDeviceSetupView(model: model)
            } else if model.isAwaitingApproval {
                AwaitingApprovalSetupView(model: model)
            } else {
                SetupWelcomeView(model: model)
            }
        } else {
            TabView(selection: $selectedTab) {
                NavigationStack {
                    DriveHomeView(model: model) {
                        selectedTab = .devices
                    }
                }
                .tabItem {
                    Label("My Drive", systemImage: "externaldrive.fill")
                }
                .tag(MainTab.drive)

                NavigationStack {
                    DevicesView(model: model)
                }
                .tabItem {
                    Label("Devices", systemImage: "person.2.fill")
                }
                .tag(MainTab.devices)

                NavigationStack {
                    SharesView(model: model)
                }
                .tabItem {
                    Label("Shares", systemImage: "person.3.fill")
                }
                .tag(MainTab.shares)

                NavigationStack {
                    BackupView(model: model)
                }
                .tabItem {
                    Label("Backup", systemImage: "arrow.triangle.2.circlepath")
                }
                .tag(MainTab.backup)

                NavigationStack {
                    SettingsView(model: model)
                }
                .tabItem {
                    Label("Settings", systemImage: "gearshape.fill")
                }
                .tag(MainTab.settings)
            }
            .onAppear {
                if model.shareDialogRequestId > 0 {
                    selectedTab = .shares
                }
            }
            .onChange(of: model.shareDialogRequestId) { _, _ in
                selectedTab = .shares
            }
        }
    }

    @MainActor
    private func revealStartupLoadingIfNeeded() async {
        showStartupLoading = false
        guard !model.stateLoaded else { return }
        do {
            try await Task.sleep(nanoseconds: 2_000_000_000)
        } catch {
            return
        }
        guard !Task.isCancelled, !model.stateLoaded else { return }
        showStartupLoading = true
    }
}

private struct CopyFeedbackToast: View {
    let message: String

    var body: some View {
        Text(message)
            .font(.callout.weight(.semibold))
            .lineLimit(1)
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .background(.regularMaterial, in: Capsule())
            .shadow(radius: 10, y: 4)
    }
}

private struct RevokedDeviceSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    VStack(spacing: 14) {
                        Image("BrandIcon")
                            .resizable()
                            .interpolation(.high)
                            .frame(width: 96, height: 96)
                        Text("Iris Drive")
                            .font(.title.bold())
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
                }

                Section("Device removed") {
                    Text("This device no longer has access to Iris Drive.")
                    LabeledContent("Device", value: model.currentAppKeyNpub)
                    LabeledContent("Current Device Key", value: model.devicePublicKey)
                    Button {
                        model.relinkDevice()
                    } label: {
                        Label("Link this device again", systemImage: "link")
                    }
                    Button {
                        model.copyDeviceKey()
                    } label: {
                        Label("Copy Device Key", systemImage: "doc.on.doc")
                    }
                }

                Section {
                    Button(role: .destructive) {
                        model.logout()
                    } label: {
                        Label("Log out", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                }
            }
            .accessibilityIdentifier("revokedDeviceView")
            .task {
                while model.isRevoked {
                    await model.refreshProfileStatusInBackground()
                    if !model.isRevoked { return }
                    try? await Task.sleep(nanoseconds: 1_000_000_000)
                    guard !Task.isCancelled else { return }
                }
            }
        }
    }
}

private struct AwaitingApprovalSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    VStack(spacing: 14) {
                        Image("BrandIcon")
                            .resizable()
                            .interpolation(.high)
                            .frame(width: 96, height: 96)
                        Text("Iris Drive")
                            .font(.title.bold())
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
                }

                Section("Waiting for approval") {
                    LabeledContent("Device", value: model.currentAppKeyNpub)
                    LabeledContent("Current Device Key", value: model.devicePublicKey)
                    Button {
                        model.copyDeviceKey()
                    } label: {
                        Label("Copy Device Key", systemImage: "doc.on.doc")
                    }
                }

                Section {
                    Button(role: .destructive) {
                        model.logout()
                    } label: {
                        Label("Log out", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                }
            }
            .accessibilityIdentifier("awaitingApprovalView")
            .task {
                while model.isAwaitingApproval {
                    await model.refreshProfileStatusInBackground()
                    if !model.isAwaitingApproval { return }
                    try? await Task.sleep(nanoseconds: 1_000_000_000)
                    guard !Task.isCancelled else { return }
                }
            }
        }
    }
}

private enum SetupRoute: Hashable {
    case create
    case photo(String)
    case restoreOptions
    case restorePhrase
    case restoreSecretKey
    case link
}

private struct SetupWelcomeView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var path: [SetupRoute] = []

    var body: some View {
        NavigationStack(path: $path) {
            Form {
                Section {
                    VStack(spacing: 14) {
                        Image("BrandIcon")
                            .resizable()
                            .interpolation(.high)
                            .frame(width: 96, height: 96)
                        Text("Iris Drive")
                            .font(.title.bold())
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
                }

                Section {
                    Button {
                        path.append(.create)
                    } label: {
                        Label("Create profile", systemImage: "plus")
                    }
                    .accessibilityIdentifier("welcomeCreateProfile")
                    Button {
                        path.append(.restoreOptions)
                    } label: {
                        Label("Sign in", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                    .accessibilityIdentifier("welcomeSignIn")
                }
            }
            .navigationDestination(for: SetupRoute.self) { route in
                switch route {
                case .create:
                    CreateProfileSetupView(model: model) { username in
                        if username.isEmpty {
                            model.createProfile(username: "", profilePhotoName: "")
                        } else {
                            path.append(.photo(username))
                        }
                    }
                case .photo(let username):
                    ProfilePhotoSetupView(model: model, username: username)
                case .restoreOptions:
                    RestoreOptionsSetupView(
                        openLinkDevice: { path.append(.link) },
                        openRecoveryPhrase: { path.append(.restorePhrase) },
                        openSecretKey: { path.append(.restoreSecretKey) }
                    )
                case .restorePhrase:
                    RestoreRecoveryPhraseSetupView(model: model)
                case .restoreSecretKey:
                    RestoreSecretKeySetupView(model: model)
                case .link:
                    LinkDeviceSetupView(model: model)
                }
            }
        }
    }
}

private struct CreateProfileSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let continueWithUsername: (String) -> Void
    @State private var username = ""

    private var trimmedUsername: String {
        username.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        Form {
            Section {
                TextField("Username (optional)", text: $username)
                    .textInputAutocapitalization(.words)
                    .accessibilityIdentifier("createUsername")
                    .onSubmit {
                        continueWithUsername(trimmedUsername)
                    }
                Button {
                    continueWithUsername(trimmedUsername)
                } label: {
                    Label(
                        trimmedUsername.isEmpty ? "Create profile" : "Continue",
                        systemImage: "plus"
                    )
                }
                .accessibilityIdentifier("createProfileSubmit")
            }
            SetupErrorSection(message: model.setupErrorMessage)
        }
        .navigationTitle("Create profile")
        .toolbar(.visible, for: .navigationBar)
    }
}

private struct ProfilePhotoSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let username: String
    @State private var selectedPhoto: PhotosPickerItem?

    var body: some View {
        Form {
            Section {
                PhotosPicker(selection: $selectedPhoto, matching: .images) {
                    Label(
                        selectedPhoto == nil ? "Choose photo" : "Photo selected",
                        systemImage: "photo"
                    )
                }
                if selectedPhoto != nil {
                    Button {
                        selectedPhoto = nil
                    } label: {
                        Label("Remove photo", systemImage: "xmark")
                    }
                }
                Button {
                    model.createProfile(
                        username: username,
                        profilePhotoName: selectedPhoto == nil ? "" : "selected-profile-photo"
                    )
                } label: {
                    Label(selectedPhoto == nil ? "Later" : "Create profile", systemImage: "plus")
                }
            }
            SetupErrorSection(message: model.setupErrorMessage)
        }
        .navigationTitle("Profile photo")
        .toolbar(.visible, for: .navigationBar)
    }
}

private struct SetupErrorSection: View {
    let message: String

    var body: some View {
        if !message.isEmpty {
            Section {
                Text(message)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("setupErrorMessage")
            }
        }
    }
}

private struct RestoreOptionsSetupView: View {
    let openLinkDevice: () -> Void
    let openRecoveryPhrase: () -> Void
    let openSecretKey: () -> Void

    var body: some View {
        Form {
            Section {
                Button(action: openLinkDevice) {
                    Label("Link device", systemImage: "link")
                }
                .accessibilityIdentifier("openLinkDevice")
                Button(action: openRecoveryPhrase) {
                    Label("Restore from recovery phrase", systemImage: "text.badge.checkmark")
                }
                .accessibilityIdentifier("openRecoveryPhrase")
                Button(action: openSecretKey) {
                    Label("Restore from secret key", systemImage: "key")
                }
                .accessibilityIdentifier("openSecretKey")
            }
        }
        .navigationTitle("Restore")
        .toolbar(.visible, for: .navigationBar)
    }
}

private struct RestoreRecoveryPhraseSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var words = Array(repeating: "", count: recoveryPhraseWordCount)
    @State private var index = 0
    @FocusState private var wordFieldFocused: Bool

    private var currentWord: Binding<String> {
        Binding(
            get: { words[index] },
            set: { value in
                applyInput(value)
            }
        )
    }

    private var phrase: String {
        words
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .joined(separator: " ")
    }

    private var currentWordIsFilled: Bool {
        !words[index].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var allWordsAreFilled: Bool {
        words.allSatisfy { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    var body: some View {
        Form {
            Section("Word \(index + 1) of \(recoveryPhraseWordCount)") {
                TextField("Word \(index + 1)", text: currentWord)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .focused($wordFieldFocused)
                    .submitLabel(index == recoveryPhraseWordCount - 1 ? .done : .next)
                    .accessibilityIdentifier("recoveryWordInput")
                    .onSubmit { advanceOrRestore() }

                Button {
                    applyInput(UIPasteboard.general.string ?? "")
                } label: {
                    Label("Paste from Clipboard", systemImage: "doc.on.clipboard")
                }
                .accessibilityIdentifier("pasteRecoveryPhrase")

                HStack {
                    Button {
                        index = max(0, index - 1)
                    } label: {
                        Label("Back", systemImage: "chevron.left")
                    }
                    .disabled(index == 0)

                    Spacer()

                    Button {
                        advanceOrRestore()
                    } label: {
                        Label(
                            index == recoveryPhraseWordCount - 1 ? "Restore" : "Next",
                            systemImage: index == recoveryPhraseWordCount - 1 ? "checkmark" : "chevron.right"
                        )
                    }
                    .disabled(index == recoveryPhraseWordCount - 1 ? !allWordsAreFilled : !currentWordIsFilled)
                    .accessibilityIdentifier(index == recoveryPhraseWordCount - 1 ? "restoreRecoveryPhraseSubmit" : "restoreRecoveryPhraseNext")
                }
            }
        }
        .navigationTitle("Recovery phrase")
        .toolbar(.visible, for: .navigationBar)
        .onAppear { wordFieldFocused = true }
    }

    private func advanceOrRestore() {
        if index == recoveryPhraseWordCount - 1 {
            guard allWordsAreFilled else { return }
            model.restoreProfile(recoverySecret: phrase)
        } else if currentWordIsFilled {
            index += 1
            wordFieldFocused = true
        }
    }

    private func applyInput(_ value: String) {
        let parts = value
            .split(whereSeparator: { $0.isWhitespace })
            .map { String($0).lowercased() }
        if parts.count <= 1 {
            words[index] = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            return
        }
        for (offset, word) in parts.enumerated() where index + offset < words.count {
            words[index + offset] = word
        }
        index = min(words.count - 1, index + parts.count - 1)
    }
}

private struct RestoreSecretKeySetupView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        Form {
            Section {
                SecureField("nsec1... or hex secret key", text: $model.restoreSecret)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit {
                        model.restoreProfile()
                    }
                    .accessibilityIdentifier("restoreSecretKeyInput")
                Button {
                    model.restoreProfile()
                } label: {
                    Label("Restore", systemImage: "key")
                }
                .disabled(model.restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityIdentifier("restoreSecretKeySubmit")
            }
        }
        .navigationTitle("Secret key")
        .toolbar(.visible, for: .navigationBar)
    }
}

private struct LinkDeviceSetupView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var linkTarget = ""
    @State private var submittedLinkTarget = ""
    @State private var scannerPresented = false

    init(model: IrisDriveMobileModel) {
        self.model = model
        _linkTarget = State(initialValue: iosUiTestDecodedValue("IRIS_DRIVE_UI_TEST_OWNER_INVITE"))
    }

    var body: some View {
        Form {
            Section {
                TextField("IrisProfile invite link or admin device key", text: $linkTarget)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("linkTargetInput")
                    .onSubmit {
                        submitLinkDevice(linkTarget, force: true)
                    }
                    .onChange(of: linkTarget) { _, newValue in
                        submitLinkDevice(newValue, force: false)
                    }
                Button {
                    submitLinkDevice(linkTarget, force: true)
                } label: {
                    Label("Link device", systemImage: "link")
                }
                .accessibilityIdentifier("linkDeviceSubmit")
                .disabled(!IrisDriveNativeLinkInput.isComplete(linkTarget.trimmingCharacters(in: .whitespacesAndNewlines)))
                Button {
                    scannerPresented = true
                } label: {
                    Label("Scan invite QR", systemImage: "qrcode.viewfinder")
                }
            }
        }
        .navigationTitle("Link device")
        .toolbar(.visible, for: .navigationBar)
        .onAppear {
            submitLinkDevice(linkTarget, force: false)
        }
        .sheet(isPresented: $scannerPresented) {
            QRCodeScannerSheet { code in
                linkTarget = code
                submitLinkDevice(code, force: false)
            }
        }
    }

    private func submitLinkDevice(_ value: String, force _: Bool) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard IrisDriveNativeLinkInput.isComplete(trimmed) else { return }
        guard submittedLinkTarget != trimmed else { return }
        submittedLinkTarget = trimmed
        model.profileLinkTarget = trimmed
        model.linkDevice()
    }
}

private struct DriveHomeView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let showDevices: () -> Void

    private var onlineDeviceCount: Int {
        model.onlineDeviceCount
    }

    private var totalDeviceCount: Int {
        model.authorizedDeviceCount
    }

    private var deviceSummaryText: String {
        "\(onlineDeviceCount)/\(totalDeviceCount) online"
    }

    var body: some View {
        List {
            Section {
                HStack(spacing: 16) {
                    Image(systemName: model.statusSymbol)
                        .font(.system(size: 40, weight: .semibold))
                        .foregroundStyle(model.statusTint)
                        .frame(width: 48)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(model.driveName)
                            .font(.title3.weight(.semibold))
                        Text(model.statusTitle)
                            .font(.headline)
                            .foregroundStyle(model.statusTint)
                        Text(model.statusDetail)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 8)
            }

            Section("Summary") {
                LabeledContent("Files", value: "\(model.fileCount)")
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("filesSummaryRow")
                    .accessibilityValue("\(model.fileCount)")
                LabeledContent("Storage", value: byteString(model.visibleFileBytes))
                Button(action: showDevices) {
                    LabeledContent("Devices", value: deviceSummaryText)
                }
                .accessibilityIdentifier("devicesSummaryButton")
                .accessibilityLabel("Devices")
                .accessibilityValue(deviceSummaryText)
            }

            Section("Files") {
                Button {
                    model.openDriveFolder()
                } label: {
                    Label("Open in Files", systemImage: "folder")
                }
                .accessibilityIdentifier("openInFilesButton")
                if !model.fileProviderError.isEmpty {
                    Text(model.fileProviderError)
                        .font(.footnote)
                        .foregroundStyle(.red)
                        .accessibilityIdentifier("openInFilesError")
                }
                Button {
                    model.openIrisApps()
                } label: {
                    Label(
                        model.isOpeningIrisApps ? "Opening Iris Apps" : "Open Iris Apps",
                        systemImage: "safari"
                    )
                }
                .disabled(!model.localNhashResolverEnabled || model.isOpeningIrisApps)
                Button {
                    model.copySnapshotLink()
                } label: {
                    Label("Copy drive.iris.to link", systemImage: "doc.on.doc")
                }
                .disabled(model.snapshotLink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button {
                    model.openSnapshotLink()
                } label: {
                    Label("View on drive.iris.to", systemImage: "safari")
                }
                .disabled(model.snapshotLink.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Sync") {
                LabeledContent("State", value: model.syncStateTitle)
                if model.syncRunning {
                    Button {
                        model.stopSync()
                    } label: {
                        Label("Pause sync", systemImage: "pause.fill")
                    }
                } else {
                    Button {
                        model.startSync()
                    } label: {
                        Label("Resume sync", systemImage: "play.fill")
                    }
                }
            }
        }
        .navigationTitle("My Drive")
    }
}

private struct DevicesView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var showingAddDevice = false
    @State private var showingAddRecoveryKey = false
    @State private var devicePendingDelete: IrisDriveDevice?

    private var deviceActors: [IrisDriveDevice] {
        model.devices.filter(\.isDeviceActor)
    }

    private var recoveryKeyActors: [IrisDriveDevice] {
        model.devices.filter { !$0.isDeviceActor }
    }

    var body: some View {
        List {
            if model.canAdminProfile {
                AddDeviceSection(model: model, isExpanded: $showingAddDevice)
            }
            Section {
                if deviceActors.isEmpty {
                    Text("No devices yet")
                        .foregroundStyle(.secondary)
                }
                ForEach(deviceActors) { device in
                    deviceRow(device, showPresence: true)
                }
            }
            if !recoveryKeyActors.isEmpty {
                Section("Recovery Keys") {
                    ForEach(recoveryKeyActors) { device in
                        deviceRow(device, showPresence: false)
                    }
                }
            }
        }
        .navigationTitle("Devices")
        .toolbar {
            if model.canAdminProfile {
                ToolbarItemGroup(placement: .primaryAction) {
                    Button {
                        showingAddRecoveryKey = true
                    } label: {
                        Label("Add Recovery Key", systemImage: "key.horizontal")
                    }
                    .accessibilityIdentifier("addRecoveryKeyButton")
                }
            }
        }
        .sheet(isPresented: $showingAddRecoveryKey) {
            AddRecoveryKeySheet(model: model, isPresented: $showingAddRecoveryKey)
        }
        .alert(
            "Remove Device?",
            isPresented: Binding(
                get: { devicePendingDelete != nil },
                set: { presented in
                    if !presented {
                        devicePendingDelete = nil
                    }
                }
            ),
            presenting: devicePendingDelete
        ) { device in
            Button("Remove", role: .destructive) {
                model.deleteDevice(id: device.id)
                devicePendingDelete = nil
            }
            Button("Cancel", role: .cancel) {
                devicePendingDelete = nil
            }
        } message: { device in
            Text("Remove \(device.label) from Iris Drive? This removes its access to future syncs.")
        }
    }

    @ViewBuilder
    private func deviceRow(_ device: IrisDriveDevice, showPresence: Bool) -> some View {
        DisclosureGroup {
            if device.detail == model.devicePublicKey {
                LabeledContent("Device Key", value: model.devicePublicKey)
            }
            Text(device.detail)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            if device.canAppointAdmin || device.canDemoteAdmin || device.canRevoke {
                HStack {
                    if device.canAppointAdmin {
                        Button {
                            model.appointAdmin(id: device.id)
                        } label: {
                            Label("Make Admin", systemImage: "person.badge.key")
                        }
                    }
                    if device.canDemoteAdmin {
                        Button {
                            model.demoteAdmin(id: device.id)
                        } label: {
                            Label("Remove Admin", systemImage: "person.badge.minus")
                        }
                    }
                    if device.canRevoke {
                        Button(role: .destructive) {
                            devicePendingDelete = device
                        } label: {
                            Label("Remove", systemImage: "trash")
                        }
                    }
                }
                .buttonStyle(.bordered)
            }
        } label: {
            HStack {
                if showPresence {
                    Image(systemName: device.isOnline ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(device.isOnline ? .green : .secondary)
                }
                VStack(alignment: .leading) {
                    Text(device.label)
                    Text(deviceSubtitle(device, includeConnection: showPresence))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .swipeActions {
            if device.canAppointAdmin {
                Button {
                    model.appointAdmin(id: device.id)
                } label: {
                    Label("Make Admin", systemImage: "person.badge.key")
                }
            }
            if device.canDemoteAdmin {
                Button {
                    model.demoteAdmin(id: device.id)
                } label: {
                    Label("Remove Admin", systemImage: "person.badge.minus")
                }
            }
            if device.canRevoke {
                Button(role: .destructive) {
                    devicePendingDelete = device
                } label: {
                    Label("Delete", systemImage: "trash")
                }
            }
        }
    }

    private func deviceSubtitle(_ device: IrisDriveDevice, includeConnection: Bool) -> String {
        var parts = [device.role, device.state].filter { !$0.isEmpty }
        if includeConnection && !device.connectionLabel.isEmpty {
            parts.append(device.connectionLabel)
        }
        return parts.joined(separator: " | ")
    }
}

private struct AddDeviceSection: View {
    @ObservedObject var model: IrisDriveMobileModel
    @Binding var isExpanded: Bool

    private var canAddManualDevice: Bool {
        IrisDriveNativeLinkInput.isComplete(model.approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    private func submitManualDevice() {
        guard canAddManualDevice else { return }
        model.approveDevice()
    }

    var body: some View {
        Section {
            DisclosureGroup(isExpanded: $isExpanded) {
                Text("Paste the device key.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                TextField("Device key", text: $model.approveDeviceKey)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("manualDeviceId")
                    .onSubmit {
                        submitManualDevice()
                    }
                TextField("Name (optional)", text: $model.approveDeviceLabel)
                    .accessibilityIdentifier("manualDeviceName")
                    .onSubmit {
                        submitManualDevice()
                    }
                Button {
                    submitManualDevice()
                } label: {
                    Label("Add", systemImage: "plus")
                }
                .accessibilityIdentifier("manualDeviceAdd")
                .disabled(!canAddManualDevice)

                if !model.appKeyLinkInvite.isEmpty {
                    QrCodeView(matrix: model.qrMatrix(for: model.appKeyLinkInvite))
                        .frame(width: 260, height: 260)
                        .frame(maxWidth: .infinity, alignment: .center)
                    Text(model.appKeyLinkInvite)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                    HStack {
                        Button {
                            model.copyLinkInvite()
                        } label: {
                            Label("Copy invite link", systemImage: "link")
                        }
                        Button {
                            model.resetInvite()
                        } label: {
                            Label("Reset invite", systemImage: "arrow.clockwise")
                        }
                    }
                }

                if !model.inboundAppKeyLinkRequests.isEmpty {
                    Text("Device requests")
                        .font(.headline)
                    ForEach(model.inboundAppKeyLinkRequests) { request in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(request.label.isEmpty ? "New device" : request.label)
                                .font(.headline)
                            Text(request.devicePubkey)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                            Button {
                                model.approveDevice(request: request.requestLink, label: request.label)
                            } label: {
                                Label("Add", systemImage: "plus")
                            }
                            Button(role: .destructive) {
                                model.rejectDevice(request: request.requestLink)
                            } label: {
                                Label("Reject", systemImage: "xmark")
                            }
                        }
                    }
                }
            } label: {
                HStack {
                    Label("Add Device", systemImage: "plus")
                    Spacer()
                    if !model.inboundAppKeyLinkRequests.isEmpty {
                        Text("\(model.inboundAppKeyLinkRequests.count) request\(model.inboundAppKeyLinkRequests.count == 1 ? "" : "s")")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .onAppear {
            prefillUiTestDeviceFields()
        }
    }

    private func prefillUiTestDeviceFields() {
        let request = iosUiTestValue("IRIS_DRIVE_UI_TEST_LINKED_DEVICE")
        if !request.isEmpty,
           model.approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            model.approveDeviceKey = request
        }

        let label = iosUiTestValue("IRIS_DRIVE_UI_TEST_LINKED_DEVICE_LABEL")
        if !label.isEmpty,
           model.approveDeviceLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            model.approveDeviceLabel = label
        }
    }
}

private struct AddRecoveryKeySheet: View {
    @ObservedObject var model: IrisDriveMobileModel
    @Binding var isPresented: Bool
    @State private var mode = "choose"
    @State private var error = ""
    @State private var generatedWords: [String] = []
    @State private var generatedPubkey = ""
    @State private var generatedWordIndex = 0
    @State private var importedWords = Array(repeating: "", count: recoveryPhraseWordCount)
    @State private var importedWordIndex = 0

    var body: some View {
        NavigationStack {
            Form {
                if !error.isEmpty {
                    Section {
                        Text(error)
                            .foregroundStyle(.red)
                    }
                }
                switch mode {
                case "generate":
                    Section("Generate New") {
                        Text("Write down each word. Iris Drive will only save the public recovery key.")
                            .foregroundStyle(.secondary)
                        Text("Word \(generatedWordIndex + 1) of \(recoveryPhraseWordCount)")
                            .font(.headline)
                        Text(generatedWords.indices.contains(generatedWordIndex) ? generatedWords[generatedWordIndex] : "")
                            .font(.largeTitle.bold())
                            .textSelection(.enabled)
                    }
                    Section {
                        Button("Back") {
                            mode = "choose"
                            error = ""
                        }
                    }
                case "import":
                    Section("Import Existing") {
                        Text("Enter the recovery phrase one word at a time.")
                            .foregroundStyle(.secondary)
                        TextField(
                            "Word \(importedWordIndex + 1) of \(recoveryPhraseWordCount)",
                            text: Binding(
                                get: { importedWords[importedWordIndex] },
                                set: {
                                    importedWords[importedWordIndex] = $0
                                        .trimmingCharacters(in: .whitespacesAndNewlines)
                                        .lowercased()
                                }
                            )
                        )
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    }
                    Section {
                        Button("Back") {
                            mode = "choose"
                            error = ""
                        }
                    }
                default:
                    Section {
                        Button("Generate New") {
                            startGenerate()
                        }
                        Button("Import Existing") {
                            startImport()
                        }
                    }
                }
            }
            .navigationTitle("Add Recovery Key")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        isPresented = false
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    confirmationButton
                }
            }
        }
    }

    @ViewBuilder
    private var confirmationButton: some View {
        switch mode {
        case "generate":
            Button(generatedWordIndex >= recoveryPhraseWordCount - 1 ? "Add Recovery Key" : "Next") {
                if generatedWordIndex >= recoveryPhraseWordCount - 1 {
                    model.addRecoveryKey(pubkey: generatedPubkey)
                    isPresented = false
                } else {
                    generatedWordIndex += 1
                }
            }
            .disabled(
                !error.isEmpty ||
                    generatedWords.count != recoveryPhraseWordCount ||
                    generatedPubkey.isEmpty
            )
        case "import":
            Button(importedWordIndex >= recoveryPhraseWordCount - 1 ? "Add Recovery Key" : "Next") {
                if importedWordIndex >= recoveryPhraseWordCount - 1 {
                    addImportedRecoveryKey()
                } else {
                    importedWordIndex += 1
                }
            }
            .disabled(
                importedWords[importedWordIndex].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                    (importedWordIndex >= recoveryPhraseWordCount - 1 &&
                        importedWords.contains { $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
            )
        default:
            EmptyView()
        }
    }

    private func startGenerate() {
        let generated = model.generateRecoveryKey()
        error = generated.error
        generatedWords = generated.words
        generatedPubkey = generated.recoveryPubkey
        generatedWordIndex = 0
        if error.isEmpty, (generatedWords.count != recoveryPhraseWordCount || generatedPubkey.isEmpty) {
            error = "Recovery key generation failed"
        }
        mode = "generate"
    }

    private func startImport() {
        importedWords = Array(repeating: "", count: recoveryPhraseWordCount)
        importedWordIndex = 0
        error = ""
        mode = "import"
    }

    private func addImportedRecoveryKey() {
        let phrase = importedWords
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .joined(separator: " ")
        let derived = model.recoveryPubkey(forPhrase: phrase)
        if !derived.error.isEmpty || derived.recoveryPubkey.isEmpty {
            error = derived.error.isEmpty ? "Recovery key import failed" : derived.error
            return
        }
        model.addRecoveryKey(pubkey: derived.recoveryPubkey)
        isPresented = false
    }
}

private func iosUiTestValue(_ name: String) -> String {
    #if DEBUG
    ProcessInfo.processInfo.environment[name] ?? ""
    #else
    ""
    #endif
}

private func iosUiTestDecodedValue(_ name: String) -> String {
    #if DEBUG
    let environment = ProcessInfo.processInfo.environment
    if let encoded = environment["\(name)_B64"],
       let data = Data(base64Encoded: encoded),
       let value = String(data: data, encoding: .utf8) {
        return value
    }
    return environment[name] ?? ""
    #else
    ""
    #endif
}

private struct SharesView: View {
    @ObservedObject var model: IrisDriveMobileModel

    var body: some View {
        Form {
            Section("Create Shared Folder") {
                TextField("Folder path", text: $model.shareSourceInput)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.createShare() }
                    .accessibilityIdentifier("shareSourceInput")
                Button {
                    model.createShare()
                } label: {
                    Label("Create Shared Folder", systemImage: "plus")
                }
                .disabled(model.shareSourceInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityIdentifier("createShareButton")
            }

            Section("Accept Invite") {
                TextField("Share invite", text: $model.shareInviteInput)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.acceptShareInvite() }
                    .accessibilityIdentifier("shareInviteInput")
                Button {
                    model.acceptShareInvite()
                } label: {
                    Label("Accept invite", systemImage: "tray.and.arrow.down.fill")
                }
                .disabled(model.shareInviteInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityIdentifier("acceptShareInviteButton")
                Button {
                    model.copyLastShareInvite()
                } label: {
                    Label("Copy last invite", systemImage: "doc.on.doc")
                }
                .disabled(model.lastShareInvite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }

            Section("Share Identity") {
                Button {
                    model.copyShareRecipientEvidence()
                } label: {
                    Label("Copy my share identity", systemImage: "person.crop.circle.badge.checkmark")
                }
                .disabled(!model.hasLocalProfile)
                .accessibilityIdentifier("copyShareIdentityButton")
            }

            Section("Shared Folders") {
                if model.shares.isEmpty {
                    Text("No shared folders")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.shares) { share in
                    ShareRow(model: model, share: share)
                }
            }
        }
        .navigationTitle("Shares")
    }
}

private struct ShareRow: View {
    @ObservedObject var model: IrisDriveMobileModel
    let share: IrisDriveShare
    @State private var showingInvite = false
    @State private var revokeTarget: IrisDriveShareMember?
    @State private var showingDeleteShare = false

    var body: some View {
        DisclosureGroup {
            LabeledContent("Role", value: share.roleLabel.isEmpty ? share.role : share.roleLabel)
            LabeledContent("Key", value: share.keyStatusLabel.isEmpty ? share.keyStatus : share.keyStatusLabel)
            if !share.sourcePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                LabeledContent("Source", value: shortText(share.sourcePath))
            }
            if let epoch = share.currentKeyEpoch {
                LabeledContent("Epoch", value: "\(epoch)")
            }
            if let shortcut = share.shortcutPaths.first {
                LabeledContent("My Drive", value: shortText(shortcut))
            }
            HStack {
                if !shareOpenPath(share).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Button {
                        model.openShareFolder(share)
                    } label: {
                        Label("Open", systemImage: "folder")
                    }
                }
                if share.canAdmin {
                    Button {
                        showingInvite = true
                    } label: {
                        Label("Invite", systemImage: "person.badge.plus")
                    }
                }
                if share.repairNeeded || !share.missingKeyWraps.isEmpty {
                    Button {
                        model.repairShareWraps(shareId: share.shareId)
                    } label: {
                        Label("Repair", systemImage: "arrow.triangle.2.circlepath")
                    }
                }
                if share.shortcutPaths.isEmpty {
                    Button {
                        model.addShareShortcut(shareId: share.shareId, displayName: shareDisplayName(share))
                    } label: {
                        Label("Add to My Drive", systemImage: "link")
                    }
                }
                Button(role: .destructive) {
                    showingDeleteShare = true
                } label: {
                    Label("Delete", systemImage: "trash")
                }
            }
            .buttonStyle(.bordered)
            ForEach(share.members) { member in
                HStack {
                    VStack(alignment: .leading) {
                        Text(memberDisplayName(member))
                        Text(memberMetadata(member))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if share.canAdmin,
                       member.status != "revoked",
                       member.profileId != model.localProfileId {
                        Button(role: .destructive) {
                            revokeTarget = member
                        } label: {
                            Label("Revoke", systemImage: "trash")
                        }
                        .labelStyle(.iconOnly)
                    }
                }
            }
            ForEach(share.pendingInvites) { invite in
                VStack(alignment: .leading) {
                    Text(pendingInviteDisplayName(invite))
                    Text(pendingInviteMetadata(invite))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        } label: {
            VStack(alignment: .leading) {
                Text(shareDisplayName(share))
                Text("\(share.participantCount) people | \(share.keyStatusLabel.isEmpty ? share.keyStatus : share.keyStatusLabel)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .sheet(isPresented: $showingInvite) {
            InviteShareMemberSheet(
                model: model,
                share: share,
                profileId: model.shareRecipientProfileId,
                representativeNpubHint: model.shareRecipientNpubHint,
                displayName: model.shareRecipientDisplayName
            )
        }
        .alert(
            "Revoke access?",
            isPresented: Binding(
                get: { revokeTarget != nil },
                set: { presented in
                    if !presented {
                        revokeTarget = nil
                    }
                }
            ),
            presenting: revokeTarget
        ) { member in
            Button("Revoke", role: .destructive) {
                model.revokeShareMember(shareId: share.shareId, profileId: member.profileId)
                revokeTarget = nil
            }
            Button("Cancel", role: .cancel) {
                revokeTarget = nil
            }
        } message: { member in
            Text("Revoke \(memberDisplayName(member)) from \(shareDisplayName(share))?")
        }
        .alert("Delete share?", isPresented: $showingDeleteShare) {
            Button("Delete", role: .destructive) {
                model.deleteShare(shareId: share.shareId)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Delete \(shareDisplayName(share)) from this device? Folder contents stay in My Drive.")
        }
    }
}

private struct InviteShareMemberSheet: View {
    @ObservedObject var model: IrisDriveMobileModel
    let share: IrisDriveShare
    @Environment(\.dismiss) private var dismiss
    @State private var evidenceJson = ""
    @State private var profileId = ""
    @State private var appKey = ""
    @State private var role = "reader"
    @State private var representativeNpubHint = ""
    @State private var displayName = ""
    @State private var label = ""

    init(
        model: IrisDriveMobileModel,
        share: IrisDriveShare,
        profileId: String = "",
        representativeNpubHint: String = "",
        displayName: String = ""
    ) {
        self.model = model
        self.share = share
        _profileId = State(initialValue: profileId)
        _representativeNpubHint = State(initialValue: representativeNpubHint)
        _displayName = State(initialValue: displayName)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Recipient") {
                    TextEditor(text: $evidenceJson)
                        .font(.system(.body, design: .monospaced))
                        .frame(minHeight: 120)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section("Advanced Device") {
                    TextField("Member profile UUID", text: $profileId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Recipient device key", text: $appKey)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Picker("Role", selection: $role) {
                        Text("Reader").tag("reader")
                        Text("Editor").tag("editor")
                        Text("Admin").tag("admin")
                    }
                }
                Section("Contact") {
                    TextField("User ID", text: $representativeNpubHint)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Name", text: $displayName)
                    TextField("Device label", text: $label)
                }
            }
            .navigationTitle("Invite")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Invite") {
                        if evidenceJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            if profileId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                               appKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                model.recordPendingShareInvite(
                                    shareId: share.shareId,
                                    representativeNpubHint: representativeNpubHint,
                                    role: role,
                                    displayName: displayName
                                )
                            } else {
                                model.inviteShareMember(
                                    shareId: share.shareId,
                                    profileId: profileId,
                                    appKey: appKey,
                                    role: role,
                                    representativeNpubHint: representativeNpubHint,
                                    displayName: displayName,
                                    label: label
                                )
                            }
                        } else {
                            model.inviteShareMemberFromEvidence(
                                shareId: share.shareId,
                                evidenceJson: evidenceJson,
                                role: role,
                                displayName: displayName
                            )
                        }
                        dismiss()
                    }
                    .disabled(!canSubmitInvite)
                }
            }
        }
    }

    private var canSubmitInvite: Bool {
        if !evidenceJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return true
        }
        let profilePresent = !profileId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let appKeyPresent = !appKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let npubPresent = !representativeNpubHint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return (profilePresent && appKeyPresent) || (!profilePresent && !appKeyPresent && npubPresent)
    }
}

private struct RecoveryPhraseExportView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var export = NativeRecoverySecretExport()
    @State private var index = 0
    @State private var copied = false

    private var currentWord: String {
        guard export.words.indices.contains(index) else { return "" }
        return export.words[index]
    }

    var body: some View {
        Form {
            if !export.error.isEmpty {
                Section {
                    Text(export.error)
                        .foregroundStyle(.secondary)
                }
            } else if export.words.count == recoveryPhraseWordCount {
                Section("Word \(index + 1) of \(recoveryPhraseWordCount)") {
                    Text(currentWord)
                        .font(.title.monospaced().bold())
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.vertical, 16)
                        .accessibilityIdentifier("recoveryPhraseWord")
                    HStack {
                        Button {
                            index = max(0, index - 1)
                        } label: {
                            Label("Back", systemImage: "chevron.left")
                        }
                        .disabled(index == 0)

                        Spacer()

                        Button {
                            index = min(recoveryPhraseWordCount - 1, index + 1)
                        } label: {
                            Label("Next", systemImage: "chevron.right")
                        }
                        .disabled(index == recoveryPhraseWordCount - 1)
                    }
                }
                Section {
                    Button {
                        model.copyToClipboard(export.recoveryPhrase, feedback: "Recovery phrase copied")
                        copied = true
                    } label: {
                        Label(copied ? "Copied" : "Copy recovery phrase", systemImage: "doc.on.doc")
                    }
                    .accessibilityIdentifier("copyRecoveryPhrase")
                    Button {
                        model.copyToClipboard(export.secretKey, feedback: "Secret key copied")
                    } label: {
                        Label("Copy secret key", systemImage: "key")
                    }
                }
            }
        }
        .navigationTitle("Recovery phrase")
        .task {
            export = model.exportRecoverySecret()
        }
    }
}

private struct IrisWebBrowserView: View {
    @ObservedObject var model: IrisDriveMobileModel
    let route: IrisWebRoute
    @StateObject private var browser = IrisWebBrowserController()
    @State private var addressText: String
    @State private var isLoading = true
    @State private var loadError = ""
    @State private var addressFocused = false

    init(model: IrisDriveMobileModel, route: IrisWebRoute) {
        self.model = model
        self.route = route
        _addressText = State(initialValue: route.url.absoluteString)
    }

    var body: some View {
        ZStack {
            IrisWebView(
                initialURL: route.url,
                model: model,
                browser: browser,
                isLoading: $isLoading,
                loadError: $loadError
            )
            .ignoresSafeArea(.container, edges: [.top, .bottom])
            if isLoading {
                ProgressView()
                    .controlSize(.large)
                    .accessibilityIdentifier("irisWebLoading")
            }
            if !loadError.isEmpty {
                VStack(spacing: 12) {
                    Image(systemName: "exclamationmark.triangle")
                        .font(.title)
                        .foregroundStyle(.orange)
                    Text("Iris Apps failed to load")
                        .font(.headline)
                    Text(loadError)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .accessibilityIdentifier("irisWebError")
                }
                .padding(24)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(.background)
            }
        }
        .overlay {
            IrisWebBrowserSafeAreaChrome()
        }
        .overlay(alignment: .bottom) {
            IrisWebBrowserBar(
                browser: browser,
                addressText: $addressText,
                addressFocused: $addressFocused,
                publisherDisplayName: model.irisWebPublisherDisplayName,
                onClose: { model.webRoute = nil },
                onSubmitAddress: loadAddressBarURL
            )
            .background {
                GeometryReader { proxy in
                    Color.clear.preference(
                        key: IrisWebBrowserFooterHeightPreferenceKey.self,
                        value: proxy.size.height
                    )
                }
            }
        }
        .onPreferenceChange(IrisWebBrowserFooterHeightPreferenceKey.self) { height in
            browser.setFooterOverlayHeight(height)
        }
        .presentationDragIndicator(.hidden)
        .task {
            model.refreshIrisWebPublisherDisplayName(for: route.url)
        }
        .onChange(of: route.url) {
            isLoading = true
            loadError = ""
            model.refreshIrisWebPublisherDisplayName(for: route.url)
        }
        .onChange(of: browser.currentURL) { _, url in
            if !addressFocused {
                addressText = url?.absoluteString ?? ""
            }
            model.refreshIrisWebPublisherDisplayName(for: url)
        }
    }

    private func loadAddressBarURL() {
        let candidate = model.browserAddressURL(addressText)
        guard let url = URL(string: candidate) else { return }
        browser.load(url)
        addressFocused = false
    }
}

private struct IrisWebBrowserSafeAreaChrome: View {
    var body: some View {
        GeometryReader { proxy in
            VStack(spacing: 0) {
                IrisWebBrowserSafeAreaStrip()
                    .frame(height: proxy.safeAreaInsets.top)
                    .ignoresSafeArea(.container, edges: .top)
                Spacer(minLength: 0)
                IrisWebBrowserSafeAreaStrip()
                    .frame(height: proxy.safeAreaInsets.bottom)
                    .ignoresSafeArea(.container, edges: .bottom)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .accessibilityHidden(true)
    }
}

private struct IrisWebBrowserSafeAreaStrip: View {
    var body: some View {
        Color(uiColor: .systemBackground)
    }
}

private struct IrisWebBrowserFooterHeightPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = irisWebBrowserExpandedFooterHeight

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        let next = nextValue()
        if next > 0 {
            value = next
        }
    }
}

@MainActor
private final class IrisWebBrowserController: ObservableObject {
    @Published var currentURL: URL?
    @Published var pageTitle = ""
    @Published var canGoBack = false
    @Published var footerCollapsed = false
    private weak var webView: WKWebView?
    private var lastScrollY: CGFloat = 0
    private var footerOverlayHeight = irisWebBrowserExpandedFooterHeight

    func attach(_ webView: WKWebView) {
        self.webView = webView
        update(from: webView)
        DispatchQueue.main.async { [weak self] in
            self?.updateBrowserInsets()
        }
    }

    func update(from webView: WKWebView) {
        currentURL = webView.url
        pageTitle = webView.title ?? ""
        canGoBack = webView.canGoBack
        updateBrowserInsets(for: webView)
    }

    func setFooterOverlayHeight(_ height: CGFloat) {
        guard height > 0, abs(footerOverlayHeight - height) > 0.5 else { return }
        footerOverlayHeight = height
        updateBrowserInsets()
    }

    func goBack() {
        setFooterCollapsed(false)
        webView?.goBack()
        if let webView {
            update(from: webView)
        }
    }

    func reload() {
        setFooterCollapsed(false)
        webView?.reload()
    }

    func load(_ url: URL) {
        setFooterCollapsed(false)
        webView?.load(URLRequest(url: url))
    }

    func expandFooter() {
        setFooterCollapsed(false)
    }

    func updateScroll(_ scrollView: UIScrollView) {
        let y = max(0, scrollView.contentOffset.y + scrollView.adjustedContentInset.top)
        let delta = y - lastScrollY
        defer { lastScrollY = y }

        guard scrollView.isDragging || scrollView.isDecelerating else { return }
        if y < 12 {
            setFooterCollapsed(false)
            return
        }
        if delta > 9 {
            setFooterCollapsed(true)
        } else if delta < -7 {
            setFooterCollapsed(false)
        }
    }

    func updateBrowserInsets() {
        updateBrowserInsets(for: nil)
    }

    private func setFooterCollapsed(_ collapsed: Bool) {
        guard footerCollapsed != collapsed else { return }
        footerCollapsed = collapsed
        updateBrowserInsets()
    }

    private func updateBrowserInsets(for candidateWebView: WKWebView?) {
        guard let webView = candidateWebView ?? webView else { return }
        let safeAreaInsets = irisWebBrowserSafeAreaInsets(for: webView)
        let fallbackHeight = footerCollapsed
            ? irisWebBrowserCollapsedFooterHeight
            : irisWebBrowserExpandedFooterHeight
        let footerHeight = max(footerOverlayHeight, fallbackHeight)
        let bottomInset = safeAreaInsets.bottom + footerHeight + irisWebBrowserFooterClearance
        let contentInset = UIEdgeInsets.zero
        if webView.scrollView.contentInset != contentInset {
            webView.scrollView.contentInset = contentInset
        }
        webView.scrollView.verticalScrollIndicatorInsets = UIEdgeInsets(
            top: safeAreaInsets.top,
            left: 0,
            bottom: bottomInset,
            right: 0
        )
    }
}

private struct IrisWebBrowserBar: View {
    @ObservedObject var browser: IrisWebBrowserController
    @Binding var addressText: String
    @Binding var addressFocused: Bool
    let publisherDisplayName: (String) -> String?
    let onClose: () -> Void
    let onSubmitAddress: () -> Void
    @Namespace private var footerNamespace
    @State private var addressEditing = false

    var body: some View {
        let collapsed = browser.footerCollapsed && !addressFocused && !addressEditing
        HStack(spacing: 0) {
            if collapsed {
                Spacer(minLength: 0)
                compactBar
                    .frame(maxWidth: 232)
                    .transition(.opacity.combined(with: .scale(scale: 0.96, anchor: .bottom)))
                Spacer(minLength: 0)
            } else {
                expandedBar
                    .transition(.opacity.combined(with: .scale(scale: 1.02, anchor: .bottom)))
            }
        }
        .padding(.horizontal, collapsed ? 44 : 16)
        .padding(.top, collapsed ? 4 : 6)
        .padding(.bottom, collapsed ? 6 : 8)
        .animation(.spring(response: 0.32, dampingFraction: 0.86), value: collapsed)
        .onChange(of: addressFocused) { _, focused in
            if focused {
                browser.expandFooter()
                addressEditing = true
                addressText = browser.currentURL?.absoluteString ?? addressText
            } else if let url = browser.currentURL {
                addressEditing = false
                addressText = url.absoluteString
            }
        }
    }

    private var expandedBar: some View {
        HStack(spacing: 9) {
            browserIconButton("xmark", label: "Close", action: onClose)
                .accessibilityIdentifier("irisWebCloseButton")

            browserIconButton("chevron.left", label: "Back", action: browser.goBack)
                .opacity(browser.canGoBack ? 1 : 0.65)
                .accessibilityIdentifier("irisWebBackButton")

            HStack(spacing: 6) {
                Group {
                    if addressEditing || addressFocused {
                        addressEditor
                    } else {
                        addressDisplayButton
                    }
                }

                Button(action: browser.reload) {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 15, weight: .semibold))
                        .frame(width: 28, height: 30)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Reload")
                .accessibilityIdentifier("irisWebReloadButton")
            }
            .padding(.leading, 14)
            .padding(.trailing, 7)
            .frame(height: 44)
            .browserFooterGlass(shape: Capsule())
            .matchedGeometryEffect(id: "irisWebAddressPill", in: footerNamespace)

            browserMenu
        }
    }

    private var addressEditor: some View {
        IrisAddressTextField(
            text: $addressText,
            isFocused: $addressFocused,
            onSubmit: onSubmitAddress
        )
        .frame(maxWidth: .infinity)
    }

    private var addressDisplayButton: some View {
        Button(action: focusAddressField) {
            Text(expandedAddressDisplayText)
                .font(.system(.subheadline, design: .rounded, weight: .regular))
                .foregroundStyle(.primary.opacity(0.84))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .frame(height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Address")
        .accessibilityValue(expandedAddressDisplayText)
        .accessibilityIdentifier("irisWebAddressField")
    }

    private var expandedAddressDisplayText: String {
        irisWebFooterDisplayTitle(
            url: browser.currentURL,
            pageTitle: browser.pageTitle,
            publisherDisplayName: publisherDisplayName
        )
    }

    private func focusAddressField() {
        browser.expandFooter()
        addressText = browser.currentURL?.absoluteString ?? addressText
        addressEditing = true
        DispatchQueue.main.async {
            addressFocused = true
        }
    }

    private var compactBar: some View {
        let title = compactTitle
        return Button(action: browser.expandFooter) {
            Text(title)
                .font(.system(size: 13, weight: .regular, design: .rounded))
                .foregroundStyle(.primary.opacity(0.84))
                .lineLimit(1)
                .minimumScaleFactor(0.82)
                .frame(maxWidth: .infinity)
                .frame(height: 32)
                .padding(.horizontal, 12)
                .browserFooterGlass(shape: Capsule(), shadowRadius: 14, shadowY: 7)
                .matchedGeometryEffect(id: "irisWebAddressPill", in: footerNamespace)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Show browser controls")
        .accessibilityValue(title)
        .accessibilityIdentifier("irisWebCompactTitle")
    }

    private var compactTitle: String {
        irisWebFooterDisplayTitle(
            url: browser.currentURL,
            pageTitle: browser.pageTitle,
            publisherDisplayName: publisherDisplayName
        )
    }

    private var browserMenu: some View {
        Menu {
            if let url = browser.currentURL {
                ShareLink(item: url) {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                Button {
                    UIApplication.shared.open(url)
                } label: {
                    Label("Open in Safari", systemImage: "safari")
                }
            }
            Button(role: .cancel, action: onClose) {
                Label("Close Browser", systemImage: "xmark")
            }
        } label: {
            ZStack {
                Circle().fill(Color.primary.opacity(0.001))
                Image(systemName: "ellipsis")
                    .font(.system(size: 17, weight: .semibold))
            }
                .frame(width: 44, height: 44)
                .browserFooterGlass(shape: Circle(), shadowRadius: 16, shadowY: 8)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .contentShape(Circle())
        .accessibilityLabel("More")
        .accessibilityIdentifier("irisWebMoreButton")
    }

    private func browserIconButton(
        _ systemName: String,
        label: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            ZStack {
                Circle().fill(Color.primary.opacity(0.001))
                Image(systemName: systemName)
                    .font(.system(size: 17, weight: .semibold))
            }
                .frame(width: 44, height: 44)
                .browserFooterGlass(shape: Circle(), shadowRadius: 16, shadowY: 8)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .contentShape(Circle())
        .accessibilityLabel(label)
    }
}

private struct IrisAddressTextField: UIViewRepresentable {
    @Binding var text: String
    @Binding var isFocused: Bool
    let onSubmit: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text, isFocused: $isFocused, onSubmit: onSubmit)
    }

    func makeUIView(context: Context) -> UITextField {
        let textField = UITextField(frame: .zero)
        textField.delegate = context.coordinator
        textField.addTarget(
            context.coordinator,
            action: #selector(Coordinator.textDidChange(_:)),
            for: .editingChanged
        )
        textField.accessibilityIdentifier = "irisWebAddressField"
        textField.accessibilityLabel = "Address"
        textField.adjustsFontForContentSizeCategory = true
        textField.autocapitalizationType = .none
        textField.autocorrectionType = .no
        textField.backgroundColor = .clear
        textField.borderStyle = .none
        textField.clearButtonMode = .never
        textField.enablesReturnKeyAutomatically = false
        textField.font = UIFontMetrics(forTextStyle: .subheadline)
            .scaledFont(for: UIFont.systemFont(ofSize: 15, weight: .regular))
        textField.keyboardType = .URL
        textField.returnKeyType = .go
        textField.textColor = .label
        textField.tintColor = .systemBlue
        textField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        textField.setContentHuggingPriority(.defaultLow, for: .horizontal)
        return textField
    }

    func updateUIView(_ textField: UITextField, context: Context) {
        context.coordinator.text = $text
        context.coordinator.isFocused = $isFocused
        context.coordinator.onSubmit = onSubmit
        if textField.text != text {
            context.coordinator.isUpdatingText = true
            textField.text = text
            context.coordinator.isUpdatingText = false
        }
        textField.accessibilityValue = text

        if isFocused {
            if textField.isFirstResponder {
                context.coordinator.selectAllIfNeeded(in: textField)
            } else {
                context.coordinator.prepareForFocusCycle()
                DispatchQueue.main.async {
                    guard context.coordinator.isFocused.wrappedValue else { return }
                    textField.becomeFirstResponder()
                    context.coordinator.selectAllIfNeeded(in: textField)
                }
            }
        } else {
            context.coordinator.resetFocusCycle()
            if textField.isFirstResponder {
                DispatchQueue.main.async {
                    guard !context.coordinator.isFocused.wrappedValue else { return }
                    textField.resignFirstResponder()
                }
            }
        }
    }

    final class Coordinator: NSObject, UITextFieldDelegate {
        var text: Binding<String>
        var isFocused: Binding<Bool>
        var onSubmit: () -> Void
        var isUpdatingText = false
        private var didSelectAllForFocusCycle = false

        init(text: Binding<String>, isFocused: Binding<Bool>, onSubmit: @escaping () -> Void) {
            self.text = text
            self.isFocused = isFocused
            self.onSubmit = onSubmit
        }

        func prepareForFocusCycle() {
            didSelectAllForFocusCycle = false
        }

        func resetFocusCycle() {
            didSelectAllForFocusCycle = false
        }

        func selectAllIfNeeded(in textField: UITextField) {
            guard !didSelectAllForFocusCycle, textField.isFirstResponder else { return }
            didSelectAllForFocusCycle = true
            DispatchQueue.main.async {
                guard textField.isFirstResponder else { return }
                textField.selectAll(nil)
            }
        }

        func textFieldDidBeginEditing(_ textField: UITextField) {
            isFocused.wrappedValue = true
            selectAllIfNeeded(in: textField)
        }

        func textFieldDidEndEditing(_ textField: UITextField) {
            isFocused.wrappedValue = false
            resetFocusCycle()
        }

        func textFieldShouldReturn(_ textField: UITextField) -> Bool {
            onSubmit()
            return false
        }

        @objc func textDidChange(_ textField: UITextField) {
            guard !isUpdatingText else { return }
            text.wrappedValue = textField.text ?? ""
        }
    }
}

private extension View {
    func browserFooterGlass<S: InsettableShape>(
        shape: S,
        shadowRadius: CGFloat = 20,
        shadowY: CGFloat = 10
    ) -> some View {
        self
            .background {
                UIKitGlassEffect(style: .systemUltraThinMaterial)
                    .opacity(0.64)
                    .clipShape(shape)
                    .allowsHitTesting(false)
            }
            .overlay {
                shape
                    .fill(.white.opacity(0.08))
                    .allowsHitTesting(false)
            }
            .overlay {
                shape
                    .stroke(.white.opacity(0.34), lineWidth: 0.55)
                    .allowsHitTesting(false)
            }
            .shadow(color: .black.opacity(0.14), radius: shadowRadius, x: 0, y: shadowY)
    }
}

private struct UIKitGlassEffect: UIViewRepresentable {
    let style: UIBlurEffect.Style

    func makeUIView(context: Context) -> UIVisualEffectView {
        let view = UIVisualEffectView(effect: UIBlurEffect(style: style))
        view.backgroundColor = .clear
        view.contentView.backgroundColor = .clear
        return view
    }

    func updateUIView(_ uiView: UIVisualEffectView, context: Context) {
        uiView.effect = UIBlurEffect(style: style)
    }
}

private func irisWebFooterDisplayTitle(
    url: URL?,
    pageTitle: String,
    publisherDisplayName: (String) -> String? = { _ in nil }
) -> String {
    let cleanTitle = pageTitle.trimmingCharacters(in: .whitespacesAndNewlines)
    guard let url,
          let host = url.host?.trimmingCharacters(in: .whitespacesAndNewlines),
          !host.isEmpty
    else {
        return cleanTitle.isEmpty ? "Browser" : cleanTitle
    }

    let lowerHost = host.lowercased()
    if let title = irisMutableSiteFooterTitle(
        url: url,
        lowerHost: lowerHost,
        publisherDisplayName: publisherDisplayName
    ) {
        return title
    }

    if isIrisHashFooterHost(lowerHost) {
        return cleanTitle.isEmpty ? "nhash" : cleanTitle
    }

    if lowerHost.hasPrefix("www.") {
        return String(lowerHost.dropFirst(4))
    }
    return lowerHost
}

private func irisMutableSiteFooterTitle(
    url: URL,
    lowerHost: String,
    publisherDisplayName: (String) -> String?
) -> String? {
    guard lowerHost == "iris.localhost" || lowerHost.hasSuffix(".iris.localhost"),
          let identity = irisWebMutableSiteIdentity(from: url)
    else {
        return nil
    }
    return "\(publisherLabel(for: identity.npub, publisherDisplayName: publisherDisplayName)) / \(identity.siteName)"
}

private func publisherLabel(
    for npub: String,
    publisherDisplayName: (String) -> String?
) -> String {
    if let name = publisherDisplayName(npub)?
        .trimmingCharacters(in: .whitespacesAndNewlines),
        !name.isEmpty {
        return name
    }
    return irisCoolName(npub)
}

private let irisCoolNameAdjectives = [
    "Amber", "Analog", "Arcane", "Astral", "Aurora", "Azure", "Blissful", "Blooming",
    "Bold", "Bright", "Brilliant", "Calm", "Celestial", "Charming", "Clear", "Clever",
    "Cosmic", "Crimson", "Crystal", "Curious", "Daring", "Deep", "Dreamy", "Electric",
    "Emerald", "Ethereal", "Fabled", "Feral", "Festival", "Floating", "Fluent", "Free",
    "Friendly", "Gentle", "Glowing", "Golden", "Graceful", "Harmonic", "Hidden", "Honey",
    "Infinite", "Kind", "Laughing", "Liminal", "Lucid", "Lunar", "Lush", "Magnetic",
    "Mellow", "Mercury", "Midnight", "Mirrored", "Mystic", "Neon", "Nimble", "Noble",
    "Northern", "Nova", "Opal", "Open", "Pacific", "Patient", "Pearl", "Playful",
    "Polished", "Prismatic", "Quiet", "Radiant", "Restless", "River", "Ruby", "Saffron",
    "Secret", "Serene", "Signal", "Silver", "Solar", "Sparkling", "Spiral", "Stellar",
    "Still", "Stormy", "Sunny", "Swift", "Tender", "Verdant", "Velvet", "Vivid",
    "Warm", "Wandering", "Wild", "Wise", "Witty", "Wonder", "Zephyr",
]

private let irisCoolNameNouns = [
    "Anchor", "Archive", "Atlas", "Aurora", "Beacon", "Bloom", "Bridge", "Canvas",
    "Cascade", "Cipher", "Circuit", "Cloud", "Comet", "Compass", "Constellation",
    "Cove", "Daydream", "Drift", "Echo", "Ember", "Field", "Festival", "Flame",
    "Flux", "Forest", "Forge", "Fountain", "Garden", "Glacier", "Halo", "Harbor",
    "Harmony", "Hearth", "Horizon", "Lantern", "Library", "Lighthouse", "Lagoon",
    "Labyrinth", "Meadow", "Melody", "Mirage", "Mosaic", "Nebula", "Nimbus", "Nova",
    "Oasis", "Opal", "Orbit", "Orchard", "Paradox", "Pearl", "Planet", "Portal",
    "Prism", "Pulse", "Quartz", "Quest", "Radiance", "Rain", "Reef", "Riddle",
    "Ripple", "River", "Sanctuary", "Satellite", "Serenade", "Signal", "Solstice",
    "Spark", "Spectrum", "Spiral", "Starlight", "Station", "Studio", "Summit",
    "Sunrise", "Tempo", "Thread", "Tide", "Trail", "Valley", "Velvet", "Voyager",
    "Vortex", "Wave", "Wonder", "Zenith",
]

private func irisCoolName(_ seed: String?) -> String {
    let trimmed = seed?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    let normalized = trimmed.isEmpty ? "iris" : trimmed
    let (first, second) = irisCoolNameHash(normalized)
    let adjective = irisCoolNameAdjectives[Int(first % UInt32(irisCoolNameAdjectives.count))]
    let noun = irisCoolNameNouns[Int(second % UInt32(irisCoolNameNouns.count))]
    return "\(adjective) \(noun)"
}

private func irisCoolNameHash(_ seed: String) -> (UInt32, UInt32) {
    var first: UInt32 = 0x811c9dc5
    var second: UInt32 = 0x85ebca6b
    for codeUnit in seed.utf16 {
        let code = UInt32(codeUnit)
        first = (first ^ code) &* 16_777_619
        second = (second ^ code) &* 2_246_822_519
    }
    return (first, second)
}

private func isIrisHashFooterHost(_ lowerHost: String) -> Bool {
    lowerHost == "nhash.iris.localhost"
        || lowerHost == "hash.localhost"
        || lowerHost.hasSuffix(".hash.localhost")
        || lowerHost.hasSuffix(".sites.iris.localhost")
}

private func irisWebBrowserSafeAreaInsets(for view: UIView) -> UIEdgeInsets {
    if let window = view.window {
        return window.safeAreaInsets
    }
    return UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }
        .flatMap(\.windows)
        .first(where: \.isKeyWindow)?
        .safeAreaInsets ?? .zero
}

private struct IrisWebView: UIViewRepresentable {
    let initialURL: URL
    @ObservedObject var model: IrisDriveMobileModel
    @ObservedObject var browser: IrisWebBrowserController
    @Binding var isLoading: Bool
    @Binding var loadError: String

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        configuration.userContentController = WKUserContentController()
        model.configureIrisWebDataStore(configuration.websiteDataStore)
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = context.coordinator
        webView.scrollView.delegate = context.coordinator
        webView.scrollView.contentInsetAdjustmentBehavior = .never
        webView.allowsBackForwardNavigationGestures = true
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.backgroundColor = .clear
        webView.scrollView.clipsToBounds = false
        if #available(iOS 15.0, *) {
            webView.underPageBackgroundColor = .clear
        }
        browser.attach(webView)
        webView.load(URLRequest(url: initialURL))
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        browser.updateBrowserInsets()
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(
            model: model,
            browser: browser,
            isLoading: $isLoading,
            loadError: $loadError
        )
    }

    final class Coordinator: NSObject, WKNavigationDelegate, UIScrollViewDelegate {
        private weak var model: IrisDriveMobileModel?
        private weak var browser: IrisWebBrowserController?
        private var isLoading: Binding<Bool>
        private var loadError: Binding<String>

        init(
            model: IrisDriveMobileModel,
            browser: IrisWebBrowserController,
            isLoading: Binding<Bool>,
            loadError: Binding<String>
        ) {
            self.model = model
            self.browser = browser
            self.isLoading = isLoading
            self.loadError = loadError
        }

        func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
            isLoading.wrappedValue = true
            loadError.wrappedValue = ""
            browser?.update(from: webView)
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            isLoading.wrappedValue = false
            loadError.wrappedValue = ""
            browser?.update(from: webView)
        }

        func webView(
            _ webView: WKWebView,
            didFail navigation: WKNavigation!,
            withError error: Error
        ) {
            browser?.update(from: webView)
            showLoadError(error)
        }

        func webView(
            _ webView: WKWebView,
            didFailProvisionalNavigation navigation: WKNavigation!,
            withError error: Error
        ) {
            browser?.update(from: webView)
            showLoadError(error)
        }

        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
            isLoading.wrappedValue = false
            loadError.wrappedValue = "Web content process terminated"
            browser?.update(from: webView)
        }

        private func showLoadError(_ error: Error) {
            let nsError = error as NSError
            if nsError.domain == NSURLErrorDomain,
               nsError.code == NSURLErrorCancelled {
                return
            }
            isLoading.wrappedValue = false
            loadError.wrappedValue = nsError.localizedDescription
        }

        func scrollViewDidScroll(_ scrollView: UIScrollView) {
            browser?.updateScroll(scrollView)
        }

        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationAction: WKNavigationAction,
            decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
        ) {
            guard let url = navigationAction.request.url else {
                decisionHandler(.allow)
                return
            }
            switch model?.irisWebNavigationAction(for: url) ?? .allow {
            case .allow:
                decisionHandler(.allow)
            case .redirect(let localURL):
                webView.load(URLRequest(url: localURL))
                decisionHandler(.cancel)
            case .handleNative(let nativeURL):
                Task { @MainActor [weak model] in
                    model?.handle(url: nativeURL)
                }
                decisionHandler(.cancel)
            case .openExternal(let externalURL):
                UIApplication.shared.open(externalURL)
                decisionHandler(.cancel)
            case .cancel:
                decisionHandler(.cancel)
            }
        }
    }
}

private struct SettingsView: View {
    @ObservedObject var model: IrisDriveMobileModel
    @State private var showingLogoutConfirmation = false

    var body: some View {
        Form {
            Section("Device") {
                TextField("Device label", text: $model.deviceLabel)
                    .onSubmit { model.persist() }
                LabeledContent("Device", value: model.currentAppKeyNpub)
                LabeledContent("Current Device Key", value: model.devicePublicKey)
                Button {
                    model.copyAppKey()
                } label: {
                    Label("Copy Device", systemImage: "doc.on.doc")
                }
                Button {
                    model.copyDeviceKey()
                } label: {
                    Label("Copy Device Key", systemImage: "doc.on.doc")
                }
                if model.canExportRecoveryPhrase {
                    NavigationLink {
                        RecoveryPhraseExportView(model: model)
                    } label: {
                        Label("Recovery phrase", systemImage: "text.badge.checkmark")
                    }
                    .accessibilityIdentifier("openRecoveryPhraseExport")
                }
                Button(role: .destructive) {
                    showingLogoutConfirmation = true
                } label: {
                    Label("Log out", systemImage: "rectangle.portrait.and.arrow.right")
                }
            }

            Section("Network") {
                ForEach(model.relayStatuses) { relay in
                    HStack {
                        Circle()
                            .fill(relayHealthColor(relay.health))
                            .frame(width: 8, height: 8)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(relay.url)
                            Text(relay.statusLabel)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button(role: .destructive) {
                            model.removeRelay(relay.url)
                        } label: {
                            Image(systemName: "trash")
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel("Remove relay")
                    }
                }
                TextField("Relay URL", text: $model.relayInput)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { model.addRelay() }
                Button {
                    model.addRelay()
                } label: {
                    Label("Add relay", systemImage: "plus")
                }
                .disabled(model.relayInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button {
                    model.resetRelays()
                } label: {
                    Label("Reset relay", systemImage: "arrow.counterclockwise")
                }
                Toggle("Sync over cellular", isOn: $model.syncOverCellular)
                    .onChange(of: model.syncOverCellular) { _, _ in
                        model.persist()
                    }
            }

            Section("Apple Calendar") {
                Toggle(
                    "Keep Apple Calendar in sync",
                    isOn: Binding(
                        get: { model.appleCalendarSyncEnabled },
                        set: { model.setAppleCalendarSyncEnabled($0) }
                    )
                )
                .accessibilityIdentifier("appleCalendarSyncToggle")
                Text(model.appleCalendarSyncStatus)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("appleCalendarSyncStatus")
            }

            Section("Advanced") {
                Button(role: .destructive) {
                    model.resetLocalState()
                } label: {
                    Label("Reset local state", systemImage: "trash")
                }
            }
        }
        .navigationTitle("Settings")
        .confirmationDialog(
            "Log out of Iris Drive on this device?",
            isPresented: $showingLogoutConfirmation,
            titleVisibility: .visible
        ) {
            Button("Log out", role: .destructive) {
                model.logout()
            }
            Button("Cancel", role: .cancel) {}
        }
    }

    private func relayHealthColor(_ health: String) -> Color {
        switch health {
        case "online":
            return .green
        case "connecting":
            return .orange
        case "error":
            return .red
        default:
            return .secondary
        }
    }
}

private struct QrCodeView: View {
    let matrix: QrMatrix

    var body: some View {
        GeometryReader { proxy in
            Canvas { context, size in
                context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.white))
                guard matrix.width > 0, matrix.cells.count >= matrix.width * matrix.width else {
                    return
                }
                let quiet = 3
                let modules = matrix.width + quiet * 2
                let cell = min(size.width, size.height) / CGFloat(modules)
                let origin = CGPoint(
                    x: (size.width - cell * CGFloat(modules)) / 2,
                    y: (size.height - cell * CGFloat(modules)) / 2
                )
                for y in 0..<matrix.width {
                    for x in 0..<matrix.width where matrix.cells[y * matrix.width + x] {
                        let rect = CGRect(
                            x: origin.x + CGFloat(x + quiet) * cell,
                            y: origin.y + CGFloat(y + quiet) * cell,
                            width: cell,
                            height: cell
                        )
                        context.fill(Path(rect), with: .color(.black))
                    }
                }
            }
            .background(.white)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .frame(width: proxy.size.width, height: proxy.size.height)
        }
    }
}

private func byteString(_ bytes: UInt64) -> String {
    ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
}

private func shareDisplayName(_ share: IrisDriveShare) -> String {
    share.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "Shared folder"
        : share.displayName
}

private func shareOpenPath(_ share: IrisDriveShare) -> String {
    if let shortcut = share.shortcutPaths.first,
       !shortcut.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        return shortcut
    }
    if !share.sourcePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        return share.sourcePath
    }
    return share.sharedWithMePath
}

private func memberDisplayName(_ member: IrisDriveShareMember) -> String {
    member.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "IrisProfile"
        : member.displayName
}

private func memberMetadata(_ member: IrisDriveShareMember) -> String {
    [
        member.roleLabel.isEmpty ? member.role : member.roleLabel,
        member.statusLabel.isEmpty ? member.status : member.statusLabel,
        shortText(member.representativeNpubHint.isEmpty ? member.profileId : member.representativeNpubHint),
    ].joined(separator: " | ")
}

private func pendingInviteDisplayName(_ invite: IrisDrivePendingShareInvite) -> String {
    invite.displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        ? "Pending contact"
        : invite.displayName
}

private func pendingInviteMetadata(_ invite: IrisDrivePendingShareInvite) -> String {
    [
        invite.roleLabel.isEmpty ? invite.role : invite.roleLabel,
        invite.statusLabel.isEmpty ? invite.status : invite.statusLabel,
        shortText(invite.representativeNpubHint),
    ].joined(separator: " | ")
}

private func shortText(_ value: String) -> String {
    guard value.count > 32 else { return value }
    return "\(value.prefix(14))...\(value.suffix(10))"
}
