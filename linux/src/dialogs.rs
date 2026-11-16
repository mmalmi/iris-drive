#[allow(clippy::wildcard_imports)]
use super::*;

/// A modal dialog over the window the user is in. The parent is read before
/// the dialog joins the application: once it has, the application reports
/// the dialog itself as its active window.
pub(crate) fn dialog_window(
    application: &impl IsA<gtk::Application>,
    title: &str,
    default_width: i32,
) -> gtk::Window {
    let parent = application.active_window();
    let dialog = gtk::Window::builder()
        .application(application)
        .title(title)
        .modal(true)
        .default_width(default_width)
        .build();
    dialog.set_transient_for(parent.as_ref());
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a display: run under Xvfb and `dbus-run-session`, since GTK asks
    /// the session bus for desktop portals at start. GTK can be started on
    /// one test thread only, so every GTK check lives in this one test. The
    /// application has no id, so it does not claim a name on the bus.
    #[test]
    fn a_dialog_is_attached_to_the_main_window_not_to_itself() {
        gtk::init().expect("GTK needs a display");
        let application = gtk::Application::builder()
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        application
            .register(None::<&gio::Cancellable>)
            .expect("application registers");
        let main_window: gtk::Window = gtk::ApplicationWindow::builder()
            .application(&application)
            .build()
            .upcast();

        let share = dialog_window(&application, "Share", 420);
        assert_eq!(share.transient_for(), Some(main_window.clone()));
        assert!(share.is_modal());
        assert_eq!(share.title().as_deref(), Some("Share"));
        assert_eq!(share.default_size().0, 420);

        share.destroy();
        let next = dialog_window(&application, "Remove access", 380);
        assert_eq!(next.transient_for(), Some(main_window));
    }
}
