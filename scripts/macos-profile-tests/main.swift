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
    NSFileProviderDomain(identifier: NSFileProviderDomainIdentifier(profile.domainIdentifier), displayName: "")
}
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
print("macOS profile isolation passed: stable domains, scoped ownership, runtime mappings, concurrent storage, snapshots, and fail-closed custom configuration")
