import Foundation

enum IrisDrivePanelTab: String, CaseIterable, Identifiable {
    case drive
    case mirrors
    case peers
    case shares
    case backup
    case settings

    var id: Self { self }

    var title: String {
        switch self {
        case .drive:
            return "Iris Drive"
        case .mirrors:
            return "Mirrors"
        case .peers:
            return "Devices"
        case .shares:
            return "Shares"
        case .backup:
            return "Backup"
        case .settings:
            return "Settings"
        }
    }

    var symbol: String {
        switch self {
        case .mirrors:
            return "folder.fill.badge.plus"
        case .drive:
            return "externaldrive.fill"
        case .peers:
            return "person.2.fill"
        case .shares:
            return "person.3.fill"
        case .backup:
            return "arrow.triangle.2.circlepath"
        case .settings:
            return "gearshape.fill"
        }
    }

    static var initialScreenshotSelection: IrisDrivePanelTab {
        guard IrisDriveScreenshotFixtures.enabled else {
            return .drive
        }
        switch IrisDriveScreenshotFixtures.tabArgument {
        case "mirrors": return .mirrors
        case "device", "devices", "peers":
            return .peers
        case "share", "shares", "share-link":
            return .shares
        case "backup", "backups", "file-server", "file-servers", "server", "servers":
            return .backup
        case "setting", "settings":
            return .settings
        default:
            return .drive
        }
    }
}
