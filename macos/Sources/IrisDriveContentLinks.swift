import AppKit

extension AppDelegate {
    func openContentLink(_ classification: [String: Any]) {
        showControlPanel()
        let rawDisplayName = (
            classification["open_display_name"] as? String
        )?.trimmingCharacters(in: .whitespacesAndNewlines)
        let label: String
        if let rawDisplayName, !rawDisplayName.isEmpty {
            label = rawDisplayName
        } else {
            label = "file"
        }
        guard classification["is_valid"] as? Bool == true,
              let urlString = classification["local_open_url"] as? String,
              let url = URL(string: urlString)
        else {
            let error = (
                classification["error"] as? String
            )?.trimmingCharacters(in: .whitespacesAndNewlines)
            if let error, !error.isEmpty {
                updateStatus(error)
            } else {
                updateStatus("Could not open \(label)")
            }
            return
        }
        updateStatus("Opening \(label)")
        NSWorkspace.shared.open(url)
    }
}
