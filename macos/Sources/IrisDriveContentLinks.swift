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
        let alert = NSAlert()
        alert.messageText = "Open \(label)?"
        alert.informativeText = "Open it now or save a copy to Iris Drive."
        alert.addButton(withTitle: "Open")
        alert.addButton(withTitle: "Save to Drive")
        alert.addButton(withTitle: "Cancel")
        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            updateStatus("Opening \(label)")
            NSWorkspace.shared.open(url)
        } else if response == .alertSecondButtonReturn {
            saveContentLink(classification, label: label)
        }
    }

    private func saveContentLink(_ classification: [String: Any], label: String) {
        guard let link = classification["normalized_input"] as? String,
              !link.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            updateStatus("Could not save \(label)")
            return
        }
        dispatchNativeAction(
            ["type": "import_content_link", "link": link],
            progress: "Saving \(label)",
            success: "Saved \(label) to Iris Drive",
            restartSyncAfterSuccess: true
        )
    }
}
