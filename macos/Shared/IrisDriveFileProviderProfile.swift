import CryptoKit
import Foundation

/// The default installation keeps its existing domain; explicit profiles own
/// separate registration, runtime handoff, and provider cache directories.
struct IrisDriveFileProviderProfile {
    let domainIdentifier: String

    init(appBaseDirectory: String?) {
        if let directory = appBaseDirectory, !directory.isEmpty {
            let path = URL(fileURLWithPath: directory, isDirectory: true)
                .standardizedFileURL.resolvingSymlinksInPath().path
            domainIdentifier = "profile-" + Self.digest(path)
        } else {
            domainIdentifier = "main"
        }
    }

    init(domainIdentifier: String) {
        self.domainIdentifier = domainIdentifier
    }

    var isDefault: Bool { domainIdentifier == "main" }

    var domainDisplayName: String {
        // Preserve the default IrisDrive root. macOS rejects a second domain
        // with that same name, even when its identifier is different.
        isDefault ? "" : domainIdentifier
    }

    var registrationIdentityKey: String {
        isDefault ? "fileProviderRegistrationIdentity"
            : "fileProviderRegistrationIdentity.\(domainIdentifier)"
    }

    func owns(_ identifier: String) -> Bool {
        identifier == domainIdentifier
    }

    func accepts(configDirectory: String) -> Bool {
        isDefault || Self(appBaseDirectory: URL(fileURLWithPath: configDirectory)
            .deletingLastPathComponent().path).domainIdentifier == domainIdentifier
    }

    func storageDirectory(in applicationSupport: URL) -> URL {
        if isDefault { return applicationSupport }
        return applicationSupport.appendingPathComponent("FileProviderDomains", isDirectory: true)
            .appendingPathComponent(Self.digest(domainIdentifier), isDirectory: true)
    }

    private static func digest(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }
}

extension IrisDriveFileProviderProfile {
    struct Runtime: Decodable {
        let configDirectory: String?
        let idriveExecutable: String?

        init(configDirectory: String?, idriveExecutable: String?) {
            self.configDirectory = configDirectory
            self.idriveExecutable = idriveExecutable
        }

        init?(userInfo: [AnyHashable: Any]?) {
            let configDirectory = userInfo?["config_dir"] as? String
            let idriveExecutable = userInfo?["idrive_executable"] as? String
            guard configDirectory != nil || idriveExecutable != nil else {
                return nil
            }
            self.init(
                configDirectory: configDirectory,
                idriveExecutable: idriveExecutable
            )
        }

        enum CodingKeys: String, CodingKey {
            case configDirectory = "config_dir"
            case idriveExecutable = "idrive_executable"
        }
    }
}
