#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn open_path(path: &PathBuf) {
    let _ = Command::new("xdg-open").arg(path).spawn();
}

pub(crate) fn open_uri(uri: &str) {
    let _ = gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>);
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
