import FileProvider
import Foundation
import UniformTypeIdentifiers

func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else { fatalError(message) }
}

let files = FileManager.default
let root = files.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
try files.createDirectory(at: root, withIntermediateDirectories: true)
defer { try? files.removeItem(at: root) }
let shared = root.appendingPathComponent("shared", isDirectory: true)
let firstBase = root.appendingPathComponent("first", isDirectory: true)
let secondBase = root.appendingPathComponent("second", isDirectory: true)
for directory in [shared, firstBase, secondBase] {
    try files.createDirectory(at: directory.appendingPathComponent("Config"), withIntermediateDirectories: true)
}
let main = IrisDriveFileProviderProfile(appBaseDirectory: nil)
let first = IrisDriveFileProviderProfile(appBaseDirectory: firstBase.path)
let second = IrisDriveFileProviderProfile(appBaseDirectory: secondBase.path)
check(main.domainIdentifier == "main", "default domain changed")
check(main.registrationIdentityKey == "fileProviderRegistrationIdentity", "default registration key changed")
check(main.storageDirectory(in: shared) == shared, "default storage directory changed")
check(IrisDriveFileProviderProfile(appBaseDirectory: "").domainIdentifier == "main", "empty override must use default")
check(first.domainIdentifier == IrisDriveFileProviderProfile(appBaseDirectory: firstBase.path + "/../first/").domainIdentifier, "normalized profile identity changed")
check(first.domainIdentifier != second.domainIdentifier, "explicit profiles share a domain")
check(!first.domainIdentifier.contains(firstBase.path), "domain leaks profile path")
check(first.registrationIdentityKey != main.registrationIdentityKey, "explicit profile overwrites default registration")
check(!first.owns(main.domainIdentifier) && !first.owns(second.domainIdentifier), "repair accepts another profile")
check(first.owns(first.domainIdentifier), "repair rejects own domain")
check(first.accepts(configDirectory: firstBase.appendingPathComponent("Config").path), "own config rejected")
check(!first.accepts(configDirectory: secondBase.appendingPathComponent("Config").path), "foreign config accepted")

func domain(_ profile: IrisDriveFileProviderProfile) -> NSFileProviderDomain {
    NSFileProviderDomain(identifier: NSFileProviderDomainIdentifier(profile.domainIdentifier),
                         displayName: profile.domainDisplayName)
}
check(domain(main).displayName.isEmpty, "default CloudStorage root name changed")
check(domain(first).displayName != domain(main).displayName, "explicit profile collides with default domain name")
check(domain(first).displayName != domain(second).displayName, "explicit profiles collide in FileProvider registration")
check(domain(first).displayName == domain(IrisDriveFileProviderProfile(appBaseDirectory: firstBase.path + "/../first/")).displayName,
      "normalized profile display name changed")
check(!domain(first).displayName.contains(firstBase.path), "display name leaks profile path")
func writeRuntime(_ profile: IrisDriveFileProviderProfile, base: URL) throws -> Data {
    let data = try JSONSerialization.data(withJSONObject: ["config_dir": base.appendingPathComponent("Config").path])
    let directory = profile.storageDirectory(in: shared)
    try files.createDirectory(at: directory, withIntermediateDirectories: true)
    try data.write(to: directory.appendingPathComponent("fileprovider-runtime.json"))
    return data
}
let mainBytes = try writeRuntime(main, base: shared)
_ = try writeRuntime(first, base: firstBase)
_ = try writeRuntime(second, base: secondBase)
let mainStorage = FileProviderStorage(domain: domain(main), applicationSupportDirectory: shared)
let firstStorage = FileProviderStorage(domain: domain(first), applicationSupportDirectory: shared)
let secondStorage = FileProviderStorage(domain: domain(second), applicationSupportDirectory: shared)
check(firstStorage.configDirectory.path == firstBase.appendingPathComponent("Config").path, "first mapping missed")
check(secondStorage.configDirectory.path == secondBase.appendingPathComponent("Config").path, "second mapping missed")
check(firstStorage.configDirectory != secondStorage.configDirectory, "last initialized domain replaced earlier runtime")
check(mainStorage.configDirectory.path == shared.appendingPathComponent("Config").path, "default mapping changed")
firstStorage.recordSnapshot(items: [.root()], anchor: NSFileProviderSyncAnchor(Data("first".utf8)))
check(!secondStorage.hasStoredSnapshot() && !mainStorage.hasStoredSnapshot(), "snapshot crossed domains")
secondStorage.recordSnapshot(items: [.trash()], anchor: NSFileProviderSyncAnchor(Data("second".utf8)))
check(firstStorage.storedSnapshotIdentifiers() == Set([NSFileProviderItemIdentifier.rootContainer.rawValue]), "second domain overwrote first snapshot")
check(secondStorage.storedSnapshotIdentifiers() == Set([NSFileProviderItemIdentifier.trashContainer.rawValue]), "second snapshot missing")
let unchangedMain = try Data(contentsOf: shared.appendingPathComponent("fileprovider-runtime.json"))
check(unchangedMain == mainBytes, "default runtime was modified")

try files.removeItem(at: first.storageDirectory(in: shared).appendingPathComponent("fileprovider-runtime.json"))
check(firstStorage.runtime == nil, "missing custom mapping fell back to default")
check(firstStorage.configDirectory != mainStorage.configDirectory, "missing custom config selected default")
var missingMappingRejected = false
do {
    _ = try firstStorage.createItem(
        template: FileProviderItem(itemIdentifier: .init("new"), parentItemIdentifier: .rootContainer,
                                   filename: "new", contentType: .folder),
        contents: nil, mayAlreadyExist: false
    )
} catch {
    missingMappingRejected = error.localizedDescription.contains("runtime is missing or invalid for this profile")
}
check(missingMappingRejected, "missing profile mapping reached provider mutation instead of failing closed")
_ = try writeRuntime(first, base: secondBase)
check(firstStorage.runtime == nil, "custom mapping accepted another profile's config")
if #available(macOS 15.0, *) {
    let wrongDomain = domain(first)
    wrongDomain.userInfo = ["config_dir": secondBase.appendingPathComponent("Config").path]
    let invalid = FileProviderStorage(domain: wrongDomain, applicationSupportDirectory: shared)
    check(invalid.runtime == nil, "domain userInfo crossed profile boundary")
    wrongDomain.userInfo = ["config_dir": firstBase.appendingPathComponent("Config").path]
    let valid = FileProviderStorage(domain: wrongDomain, applicationSupportDirectory: shared)
    check(valid.configDirectory.path == firstBase.appendingPathComponent("Config").path, "valid userInfo was rejected")
}
let unavailableMainConfig = root.appendingPathComponent("unavailable/Config")
let unavailableMainBytes = try JSONSerialization.data(withJSONObject: ["config_dir": unavailableMainConfig.path])
try unavailableMainBytes.write(to: shared.appendingPathComponent("fileprovider-runtime.json"))
check(mainStorage.configDirectory.path == unavailableMainConfig.path, "default runtime lookup behavior changed")

// "Share with Iris Drive…" in Finder: the extension's link carries the folder
// and a one-time token only that profile's app accepts.
let shareStorage = first.storageDirectory(in: shared)
let shareLink = try IrisDriveFinderShare.link(path: "Photos/Trips & more+1/Åland", storage: shareStorage)
check(shareLink.absoluteString.hasPrefix("iris-drive://share?path=Photos%2FTrips%20%26%20more%2B1%2F%C3%85land&request="),
      "Finder share link does not carry the folder")
check(!IrisDriveFinderShare.accept(shareLink, storage: second.storageDirectory(in: shared)), "Finder share request crossed profiles")
check(!IrisDriveFinderShare.accept(URL(string: "iris-drive://share?path=Photos")!, storage: shareStorage),
      "a plain share link passed as a Finder request")
check(!IrisDriveFinderShare.accept(URL(string: "iris-drive://share?path=Photos&request=guess")!, storage: shareStorage),
      "a guessed token passed as a Finder request")
check(IrisDriveFinderShare.accept(shareLink, storage: shareStorage), "Finder share request rejected")
check(!IrisDriveFinderShare.accept(shareLink, storage: shareStorage), "Finder share request accepted twice")
func folder(_ name: String, canShare: Bool) -> FileProviderItem {
    FileProviderItem(itemIdentifier: .init(name), parentItemIdentifier: .rootContainer, filename: name,
                     contentType: .folder, canShare: canShare)
}
let plainFolder = folder("Plain", canShare: false)
let shareFolder = folder("Trip", canShare: true)
check(plainFolder.itemVersion.metadataVersion != folder("Plain", canShare: true).itemVersion.metadataVersion,
      "Finder does not learn that a folder became shareable")
let extensionInfo = try PropertyListSerialization.propertyList(
    from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])), format: nil
) as? [String: Any]
let actions = (extensionInfo?["NSExtension"] as? [String: Any])?["NSExtensionFileProviderActions"] as? [[String: String]] ?? []
check(actions.count == 1 && actions[0]["NSExtensionFileProviderActionIdentifier"] == IrisDriveFinderShare.actionIdentifier,
      "Finder share action is not declared")
let shareRule = NSPredicate(format: actions[0]["NSExtensionFileProviderActionActivationRule"] ?? "FALSEPREDICATE")
func offersShare(_ items: [FileProviderItem]) -> Bool { shareRule.evaluate(with: ["fileproviderItems": items]) }
check(offersShare([shareFolder]), "Share is not offered on a folder that can be shared")
check(!offersShare([plainFolder]), "Share is offered on a folder inside or around a share")
check(!offersShare([shareFolder, folder("Other", canShare: true)]), "Share is offered on several folders at once")
print("macOS profile isolation passed: stable domains, scoped ownership, runtime mappings, concurrent storage, snapshots, and fail-closed custom configuration")
