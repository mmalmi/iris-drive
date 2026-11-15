import Foundation

/// "Share with Iris Drive…" on a folder in Finder. The File Provider extension
/// opens the app with the usual `iris-drive://share` link and leaves a one-time
/// token in the app group, so the app can tell the person's own click from a
/// share link opened by a web page or another app.
enum IrisDriveFinderShare {
    static let actionIdentifier = "to.iris.drive.share"
    /// Set on the folders Finder offers the action for; the activation rule in
    /// the extension's Info.plist reads it.
    static let itemUserInfoKey = "irisDriveCanShare"
    private static let tokenFileName = "finder-share-request"
    private static let unreserved = CharacterSet(
        charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
    )

    /// Where the extension and the app both keep the token for `profile`.
    static func storage(for profile: IrisDriveFileProviderProfile) -> URL {
        profile.storageDirectory(
            in: IrisDriveAppGroup.applicationSupportDirectory(
                teamIdentifier: IrisDriveCodeSigning.currentTeamIdentifier()
            )
        )
    }

    /// The link that opens the app on the folder at `path`.
    static func link(path: String, storage: URL) throws -> URL {
        let token = UUID().uuidString
        guard let encoded = path.addingPercentEncoding(withAllowedCharacters: unreserved),
              let url = URL(string: "iris-drive://share?path=\(encoded)&request=\(token)")
        else {
            throw CocoaError(.fileReadInvalidFileName)
        }
        try FileManager.default.createDirectory(at: storage, withIntermediateDirectories: true)
        try Data(token.utf8).write(to: storage.appendingPathComponent(tokenFileName), options: .atomic)
        return url
    }

    /// Whether `url` is the link the extension just made. A token works once.
    static func accept(_ url: URL, storage: URL) -> Bool {
        let file = storage.appendingPathComponent(tokenFileName)
        guard let token = URLComponents(url: url, resolvingAgainstBaseURL: false)?
            .queryItems?.first(where: { $0.name == "request" })?.value,
            !token.isEmpty,
            let stored = try? String(contentsOf: file, encoding: .utf8),
            stored == token
        else {
            return false
        }
        try? FileManager.default.removeItem(at: file)
        return true
    }
}
