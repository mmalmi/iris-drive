#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn check_backup_target(model: &AppRef, target: String) {
    start_backup_check(model, vec![target], "Backup checked");
}

pub(crate) fn check_backups(model: &AppRef) {
    let targets = match desktop_state() {
        Ok(state) => state
            .ui
            .backups
            .into_iter()
            .map(|backup| backup.target.trim().to_string())
            .filter(|target| !target.is_empty())
            .collect::<Vec<_>>(),
        Err(error) => {
            model.ui.notice.set_text(&error);
            return;
        }
    };
    start_backup_check(model, targets, "Backups checked");
}

fn start_backup_check(model: &AppRef, targets: Vec<String>, success: &str) {
    if model.backup_checking.get() {
        return;
    }
    let targets = targets
        .into_iter()
        .map(|target| target.trim().to_string())
        .filter(|target| !target.is_empty())
        .collect::<Vec<_>>();
    if targets.is_empty() {
        model.ui.notice.set_text("No backup targets");
        return;
    }
    model.backup_checking.set(true);
    model
        .ui
        .notice
        .set_text(&format!("Checking 0 of {}", targets.len()));
    let sender = model.backup_check_sender.clone();
    let success = success.to_string();
    std::thread::spawn(move || {
        let total = targets.len();
        for (index, target) in targets.into_iter().enumerate() {
            let _ = sender.send(BackupCheckEvent::Progress {
                checked: index,
                total,
            });
            if let Err(error) = dispatch_desktop_action(NativeAppAction::CheckBackups { target }) {
                let _ = sender.send(BackupCheckEvent::Finished(Err(error)));
                return;
            }
            let _ = sender.send(BackupCheckEvent::Progress {
                checked: index + 1,
                total,
            });
        }
        let _ = sender.send(BackupCheckEvent::Finished(Ok(success)));
    });
}
