#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn open_path(path: &PathBuf) {
    let _ = Command::new("xdg-open").arg(path).spawn();
}

pub(crate) fn open_uri(uri: &str) {
    let _ = gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>);
}

pub(crate) const HIDDEN_LAUNCH_ARG: &str = "--hidden";

pub(crate) fn configure_launch_on_startup(enabled: bool) -> Result<(), String> {
    let Some(path) = autostart_desktop_path() else {
        return Err("Autostart directory unavailable".to_owned());
    };
    if enabled {
        let executable =
            std::env::current_exe().map_err(|error| format!("Could not find app executable: {error}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create autostart directory: {error}"))?;
        }
        std::fs::write(&path, autostart_desktop_entry(&executable))
            .map_err(|error| format!("Could not write autostart entry: {error}"))?;
    } else if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("Could not remove autostart entry: {error}"))?;
    }
    Ok(())
}

fn autostart_desktop_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|config| config.join("autostart").join("to.iris.drive.desktop"))
}

fn autostart_desktop_entry(executable: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Iris Drive\nExec={} {}\nIcon=iris-drive\nTerminal=false\nCategories=Utility;Network;\nX-GNOME-Autostart-enabled=true\n",
        desktop_exec_escape(&executable.to_string_lossy()),
        HIDDEN_LAUNCH_ARG,
    )
}

fn desktop_exec_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            ' ' | '\t'
                | '\n'
                | '"'
                | '\''
                | '\\'
                | '>'
                | '<'
                | '~'
                | '|'
                | '&'
                | ';'
                | '$'
                | '*'
                | '?'
                | '#'
                | '('
                | ')'
                | '`'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

pub(crate) fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        r#"
        .iris-sidebar-button {
          border-radius: 6px;
          padding: 6px 8px;
        }
        .iris-sidebar-button label {
          font-weight: 400;
        }
        .iris-sidebar-button.selected label {
          font-weight: 700;
        }
        .iris-sidebar-action-button {
          margin: 4px 0;
        }
        .iris-sidebar-summary {
          padding: 2px 8px;
        }
        .iris-actions flowboxchild,
        .iris-metrics flowboxchild {
          padding: 0;
        }
        .iris-actions button {
          border-radius: 6px;
          padding: 3px 10px;
        }
        .iris-status-pill {
          border-radius: 999px;
          padding: 5px 9px;
          font-size: 0.82em;
          font-weight: 700;
        }
        .iris-metrics {
          margin-top: 4px;
        }
        .iris-metric-card {
          padding: 16px 12px;
          border-radius: 8px;
        }
        .iris-metric-value {
          font-size: 1.35em;
          font-weight: 700;
        }
        .iris-summary {
          padding: 12px;
          border-radius: 8px;
        }
        .iris-field-name {
          font-size: 0.92em;
        }
        .iris-value {
          font-weight: 600;
        }
        .iris-section-title {
          font-weight: 700;
        }
        .iris-drive-list {
          border-radius: 8px;
        }
        .iris-peer-online,
        .iris-peer-offline {
          min-width: 10px;
          min-height: 10px;
          border-radius: 999px;
        }
        .iris-peer-online {
          background: @success_color;
        }
        .iris-peer-offline {
          background: alpha(@window_fg_color, 0.24);
        }
        .iris-row-title {
          font-weight: 700;
        }
        "#,
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_desktop_entry_launches_gui_hidden() {
        let entry = autostart_desktop_entry(Path::new("/opt/Iris Drive/iris-drive"));

        assert!(entry.contains("Name=Iris Drive\n"));
        assert!(entry.contains("Exec=/opt/Iris\\ Drive/iris-drive --hidden\n"));
    }
}
