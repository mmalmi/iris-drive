use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use serde_json::Value;

const APP_ID: &str = "to.iris.drive";

thread_local! {
    static TRAY_APP_HOLD: RefCell<Option<gio::ApplicationHoldGuard>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct Ui {
    sidebar: gtk::Box,
    setup: gtk::Box,
    main_view: gtk::ScrolledWindow,
    main: gtk::Box,
    drive_title: gtk::Label,
    drive_message: gtk::Label,
    status_pill: gtk::Label,
    status: gtk::Label,
    folder: gtk::Label,
    owner: gtk::Label,
    device: gtk::Label,
    snapshot: gtk::Label,
    files: gtk::Label,
    blocks: gtk::Label,
    storage: gtk::Label,
    devices: gtk::Label,
    account_owner: gtk::Label,
    account_device: gtk::Label,
    account_authorization: gtk::Label,
    approve_box: gtk::Box,
    approve_device_entry: gtk::Entry,
    approve_label_entry: gtk::Entry,
    approve_device_button: gtk::Button,
    notice: gtk::Label,
    drives: gtk::ListBox,
    peers: gtk::ListBox,
    relays: gtk::ListBox,
    blossom: gtk::ListBox,
    config_path: gtk::Label,
    blocks_path: gtk::Label,
    drive_path: gtk::Label,
    root_path: gtk::Label,
    tray_on_close: gtk::CheckButton,
    relay_entry: gtk::Entry,
    init_button: gtk::Button,
    folder_button: gtk::Button,
    copy_snapshot_button: gtk::Button,
    open_snapshot_button: gtk::Button,
    restart_button: gtk::Button,
    start_button: gtk::Button,
    stop_button: gtk::Button,
}

struct AppModel {
    application: adw::Application,
    ui: Ui,
    daemon: RefCell<Option<Child>>,
    setup_screen: RefCell<SetupScreen>,
    tray: RefCell<Option<TrayServiceHandle>>,
    tray_available: Cell<bool>,
    closed_to_tray: Cell<bool>,
    retired: Cell<bool>,
    quit_requested: Cell<bool>,
}

type AppRef = Rc<AppModel>;

#[cfg(target_os = "linux")]
type TrayServiceHandle = ksni::blocking::Handle<IrisDriveTray>;

#[cfg(not(target_os = "linux"))]
type TrayServiceHandle = ();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupScreen {
    Welcome,
    Create,
    Restore,
    Link,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrayCommand {
    Show,
    OpenDriveFolder,
    StartSync,
    StopSync,
    Quit,
}

fn main() -> glib::ExitCode {
    let _app_lock = match AppInstanceLock::acquire() {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("{error}");
            return glib::ExitCode::SUCCESS;
        }
    };

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name("iris-drive");
        install_css();
    });
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Iris Drive")
        .default_width(900)
        .default_height(552)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Iris Drive", "")));
    root.append(&header);

    let folder_button = action_button("folder-open-symbolic", "Drive", "Open drive folder");
    let copy_snapshot_button = action_button(
        "insert-link-symbolic",
        "Copy Snapshot",
        "Copy snapshot link",
    );
    let open_snapshot_button =
        action_button("document-open-symbolic", "Open Snapshot", "Open snapshot");
    let init_button = text_button("Initialize");
    let restart_button = action_button("view-refresh-symbolic", "Restart", "Restart sync");
    let stop_button = action_button("process-stop-symbolic", "Stop", "Stop sync");
    let start_button = action_button("media-playback-start-symbolic", "Start", "Start sync");

    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shell.add_css_class("iris-shell");
    shell.set_hexpand(true);
    shell.set_vexpand(true);

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    sidebar.add_css_class("iris-sidebar");
    sidebar.set_width_request(128);
    sidebar.set_hexpand(false);
    sidebar.set_halign(gtk::Align::Start);
    sidebar.set_margin_top(18);
    sidebar.set_margin_bottom(18);
    sidebar.set_margin_start(8);
    sidebar.set_margin_end(8);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("iris-content");
    content.set_hexpand(true);
    content.set_vexpand(true);

    let setup = gtk::Box::new(gtk::Orientation::Vertical, 18);
    setup.set_hexpand(true);
    setup.set_vexpand(true);

    let main_view = gtk::ScrolledWindow::new();
    main_view.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    main_view.set_min_content_width(260);
    main_view.set_min_content_height(220);
    main_view.set_propagate_natural_width(false);
    main_view.set_propagate_natural_height(false);
    main_view.set_hexpand(true);
    main_view.set_vexpand(true);

    let main = gtk::Box::new(gtk::Orientation::Vertical, 20);
    main.set_margin_top(24);
    main.set_margin_bottom(24);
    main.set_margin_start(24);
    main.set_margin_end(24);
    main.set_hexpand(true);
    main.set_vexpand(true);

    let stack = gtk::Stack::new();
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    let actions = flow_section("iris-actions", 1, 6);
    actions.append(&folder_button);
    actions.append(&copy_snapshot_button);
    actions.append(&open_snapshot_button);
    actions.append(&restart_button);
    actions.append(&stop_button);
    actions.append(&start_button);
    main.append(&actions);

    let dashboard = page_box();
    dashboard.set_hexpand(true);
    dashboard.set_vexpand(true);

    let drive_header = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    drive_header.set_hexpand(true);
    let drive_icon = gtk::Image::from_icon_name("drive-harddisk-symbolic");
    drive_icon.add_css_class("iris-drive-icon");
    drive_icon.set_pixel_size(42);
    drive_header.append(&drive_icon);

    let drive_labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    drive_labels.set_hexpand(true);
    let drive_title = gtk::Label::new(Some("My Drive"));
    drive_title.add_css_class("title-2");
    drive_title.set_xalign(0.0);
    let drive_message = gtk::Label::new(Some("Starting sync"));
    drive_message.add_css_class("iris-muted");
    drive_message.set_xalign(0.0);
    drive_labels.append(&drive_title);
    drive_labels.append(&drive_message);
    drive_header.append(&drive_labels);

    let status_pill = gtk::Label::new(Some("Stopped"));
    status_pill.add_css_class("iris-status-pill");
    drive_header.append(&status_pill);
    dashboard.append(&drive_header);

    let status = value_label();
    let folder = value_label();
    let owner = value_label();
    let device = value_label();
    let snapshot = value_label();
    let files = metric_value_label();
    let blocks = metric_value_label();
    let storage = metric_value_label();
    let devices = metric_value_label();

    let metrics = flow_section("iris-metrics", 1, 4);
    metrics.append(&metric_tile("Files", &files));
    metrics.append(&metric_tile("Blocks", &blocks));
    metrics.append(&metric_tile("Storage", &storage));
    metrics.append(&metric_tile("Devices", &devices));
    dashboard.append(&metrics);

    let drives = gtk::ListBox::new();
    drives.add_css_class("iris-drive-list");
    drives.set_selection_mode(gtk::SelectionMode::None);

    let notice = gtk::Label::new(None);
    notice.add_css_class("iris-notice");
    notice.set_xalign(0.0);
    notice.set_wrap(true);
    dashboard.append(&notice);

    let peers_page = page_box();
    peers_page.append(&section_title("Devices"));
    let account_owner = value_label();
    let account_device = value_label();
    let account_authorization = value_label();
    let copy_owner_button = icon_button("edit-copy-symbolic", "Copy owner key");
    let copy_device_button = icon_button("edit-copy-symbolic", "Copy device key");
    let account_grid = gtk::Grid::new();
    account_grid.add_css_class("iris-summary");
    account_grid.set_column_spacing(10);
    account_grid.set_row_spacing(8);
    account_grid.set_hexpand(true);
    add_copy_field(&account_grid, 0, "Owner", &account_owner, &copy_owner_button);
    add_copy_field(
        &account_grid,
        1,
        "This device",
        &account_device,
        &copy_device_button,
    );
    add_field(&account_grid, 2, 0, "State", &account_authorization);
    peers_page.append(&account_grid);

    let approve_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    approve_box.set_hexpand(true);
    approve_box.append(&field_title("Approve device"));
    let approve_device_entry = setup_entry("Device request link");
    approve_device_entry.set_hexpand(true);
    let approve_label_entry = setup_entry("Device label");
    approve_label_entry.set_hexpand(true);
    let approve_device_button = action_button("emblem-ok-symbolic", "Approve", "Approve device");
    approve_box.append(&approve_device_entry);
    approve_box.append(&approve_label_entry);
    approve_box.append(&approve_device_button);
    peers_page.append(&approve_box);

    peers_page.append(&field_title("Authorized"));
    let peers = gtk::ListBox::new();
    peers.add_css_class("iris-drive-list");
    peers.set_selection_mode(gtk::SelectionMode::None);
    peers_page.append(&peers);

    let network_page = page_box();
    network_page.append(&section_title("Network"));
    let blossom = gtk::ListBox::new();
    blossom.add_css_class("iris-drive-list");
    blossom.set_selection_mode(gtk::SelectionMode::None);
    network_page.append(&endpoint_group("Blossom", &blossom));

    let relays_title = gtk::Label::new(Some("Relays"));
    relays_title.add_css_class("iris-field-name");
    relays_title.set_xalign(0.0);
    network_page.append(&relays_title);
    let relays = gtk::ListBox::new();
    relays.add_css_class("iris-drive-list");
    relays.set_selection_mode(gtk::SelectionMode::None);
    network_page.append(&relays);
    let relay_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let relay_entry = setup_entry("wss://relay.example");
    relay_entry.set_hexpand(true);
    let add_relay_button = icon_button("list-add-symbolic", "Add relay");
    let reset_relays_button = icon_button("edit-undo-symbolic", "Reset relays");
    relay_controls.append(&relay_entry);
    relay_controls.append(&add_relay_button);
    relay_controls.append(&reset_relays_button);
    network_page.append(&relay_controls);

    let hashtree_page = page_box();
    hashtree_page.append(&section_title("Hashtree"));
    let paths = gtk::Grid::new();
    paths.add_css_class("iris-summary");
    paths.set_column_spacing(12);
    paths.set_row_spacing(10);
    let config_path = value_label();
    let blocks_path = value_label();
    let drive_path = value_label();
    let root_path = value_label();
    add_field(&paths, 0, 0, "Config", &config_path);
    add_field(&paths, 1, 0, "Blocks", &blocks_path);
    add_field(&paths, 2, 0, "Drive", &drive_path);
    add_field(&paths, 3, 0, "Root", &root_path);
    hashtree_page.append(&paths);

    let settings_page = page_box();
    settings_page.append(&section_title("Settings"));
    let tray_on_close = gtk::CheckButton::with_label("Tray on close");
    tray_on_close.add_css_class("iris-setting-check");
    tray_on_close.set_active(read_close_to_tray_on_close());
    tray_on_close.set_sensitive(false);
    settings_page.append(&tray_on_close);

    stack.add_titled(&dashboard, Some("drive"), "My Drive");
    stack.add_titled(&peers_page, Some("devices"), "Devices");
    stack.add_titled(&network_page, Some("network"), "Network");
    stack.add_titled(&hashtree_page, Some("hashtree"), "Hashtree");
    stack.add_titled(&settings_page, Some("settings"), "Settings");

    let nav_items = [
        ("drive", "drive-harddisk-symbolic", "My Drive"),
        ("devices", "system-users-symbolic", "Devices"),
        ("network", "network-workgroup-symbolic", "Network"),
        ("hashtree", "network-server-symbolic", "Hashtree"),
        ("settings", "preferences-system-symbolic", "Settings"),
    ];
    let mut nav_buttons = Vec::new();
    for (name, icon, label) in nav_items {
        let button = sidebar_button(icon, label);
        let stack_for_button = stack.clone();
        button.connect_clicked(move |_| stack_for_button.set_visible_child_name(name));
        sidebar.append(&button);
        nav_buttons.push((name.to_string(), button));
    }
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    sidebar.append(&spacer);
    update_sidebar_selection(&stack, &nav_buttons);
    {
        let nav_buttons = nav_buttons.clone();
        stack.connect_visible_child_name_notify(move |stack| {
            update_sidebar_selection(stack, &nav_buttons);
        });
    }

    main.append(&stack);

    main_view.set_child(Some(&main));
    content.append(&setup);
    content.append(&main_view);
    shell.append(&sidebar);
    let content_separator = gtk::Separator::new(gtk::Orientation::Vertical);
    content_separator.add_css_class("iris-content-separator");
    shell.append(&content_separator);
    shell.append(&content);
    root.append(&shell);
    window.set_content(Some(&root));

    let model = Rc::new(AppModel {
        application: app.clone(),
        ui: Ui {
            sidebar,
            setup,
            main_view,
            main,
            drive_title,
            drive_message,
            status_pill,
            status,
            folder,
            owner,
            device,
            snapshot,
            files,
            blocks,
            storage,
            devices,
            account_owner,
            account_device,
            account_authorization,
            approve_box,
            approve_device_entry,
            approve_label_entry,
            approve_device_button,
            notice,
            drives,
            peers,
            relays,
            blossom,
            config_path,
            blocks_path,
            drive_path,
            root_path,
            tray_on_close,
            relay_entry,
            init_button,
            folder_button,
            copy_snapshot_button,
            open_snapshot_button,
            restart_button,
            start_button,
            stop_button,
        },
        daemon: RefCell::new(None),
        setup_screen: RefCell::new(SetupScreen::Welcome),
        tray: RefCell::new(None),
        tray_available: Cell::new(false),
        closed_to_tray: Cell::new(false),
        retired: Cell::new(false),
        quit_requested: Cell::new(false),
    });

    {
        let button = model.ui.init_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| initialize_drive(&model));
    }
    {
        let button = model.ui.start_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| start_daemon(&model));
    }
    {
        let button = model.ui.restart_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| restart_daemon(&model));
    }
    {
        let button = model.ui.stop_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| stop_daemon(&model));
    }
    {
        let button = model.ui.folder_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| open_drive_folder(&model));
    }
    {
        let button = model.ui.copy_snapshot_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| copy_snapshot_link(&model));
    }
    {
        let button = model.ui.open_snapshot_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| open_snapshot_link(&model));
    }
    {
        let model = Rc::clone(&model);
        copy_owner_button.connect_clicked(move |_| copy_account_key(&model, "owner_npub"));
    }
    {
        let model = Rc::clone(&model);
        copy_device_button.connect_clicked(move |_| copy_account_key(&model, "device_npub"));
    }
    {
        let button = model.ui.approve_device_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| approve_device(&model));
    }
    {
        let model = Rc::clone(&model);
        add_relay_button.connect_clicked(move |_| add_relay(&model));
    }
    {
        let model = Rc::clone(&model);
        reset_relays_button.connect_clicked(move |_| reset_relays(&model));
    }
    model.ui.tray_on_close.connect_toggled(|button| {
        write_close_to_tray_on_close(button.is_active());
    });

    connect_tray(&model, &window);

    {
        let model = Rc::clone(&model);
        glib::timeout_add_seconds_local(5, move || {
            if model.retired.get() {
                return glib::ControlFlow::Break;
            }
            if model.ui.main_view.is_visible() {
                refresh(&model);
            }
            glib::ControlFlow::Continue
        });
    }

    {
        let model = Rc::clone(&model);
        window.connect_close_request(move |window| {
            if close_to_tray_enabled(&model) {
                model.closed_to_tray.set(true);
                let window = window.clone();
                glib::idle_add_local_once(move || window.minimize());
                return glib::Propagation::Stop;
            }

            model.quit_requested.set(true);
            window.set_hide_on_close(false);
            shutdown_tray(&model);
            stop_daemon(&model);
            glib::Propagation::Proceed
        });
    }

    {
        let model = Rc::clone(&model);
        window.connect_unrealize(move |_| {
            if close_to_tray_enabled(&model) {
                model.closed_to_tray.set(true);
            }
        });
    }

    {
        let model = Rc::clone(&model);
        window.connect_unmap(move |_| {
            if close_to_tray_enabled(&model) {
                model.closed_to_tray.set(true);
            }
        });
    }

    refresh(&model);
    window.present();
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_size_request(32, 32);
    button.set_tooltip_text(Some(tooltip));
    button
}

fn text_button(label: &str) -> gtk::Button {
    gtk::Button::with_label(label)
}

fn action_button(icon: &str, label: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_tooltip_text(Some(tooltip));
    let content = adw::ButtonContent::builder()
        .icon_name(icon)
        .label(label)
        .build();
    button.set_child(Some(&content));
    button
}

fn flow_section(css_class: &str, min_children: u32, max_children: u32) -> gtk::FlowBox {
    let flow = gtk::FlowBox::new();
    flow.add_css_class(css_class);
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_activate_on_single_click(false);
    flow.set_min_children_per_line(min_children);
    flow.set_max_children_per_line(max_children);
    flow.set_column_spacing(12);
    flow.set_row_spacing(12);
    flow.set_hexpand(true);
    flow
}

fn sidebar_button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("iris-sidebar-button");
    button.set_halign(gtk::Align::Fill);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.set_hexpand(true);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(16);
    row.append(&image);
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    text.set_hexpand(true);
    row.append(&text);
    button.set_child(Some(&row));
    button
}

fn update_sidebar_selection(stack: &gtk::Stack, buttons: &[(String, gtk::Button)]) {
    let visible = stack.visible_child_name();
    for (name, button) in buttons {
        if visible.as_deref() == Some(name.as_str()) {
            button.add_css_class("selected");
        } else {
            button.remove_css_class("selected");
        }
    }
}

fn page_box() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_hexpand(true);
    page.set_vexpand(true);
    page
}

fn section_title(title: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(title));
    label.add_css_class("iris-section-title");
    label.set_xalign(0.0);
    label
}

fn field_title(title: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(title));
    label.add_css_class("iris-field-name");
    label.set_xalign(0.0);
    label
}

fn pill_button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("pill");
    button.set_height_request(44);
    button
}

fn primary_button(label: &str) -> gtk::Button {
    let button = pill_button(label);
    button.add_css_class("suggested-action");
    button
}

fn setup_entry(placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_height_request(40);
    entry
}

fn readonly_entry(value: &str) -> gtk::Entry {
    let entry = setup_entry("");
    entry.set_text(value);
    entry.set_editable(false);
    entry.set_hexpand(true);
    entry
}

fn value_label() -> gtk::Label {
    let label = gtk::Label::new(Some("..."));
    label.add_css_class("iris-value");
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::Char);
    label.set_max_width_chars(44);
    label
}

fn metric_value_label() -> gtk::Label {
    let label = gtk::Label::new(Some("0"));
    label.add_css_class("iris-metric-value");
    label.set_xalign(0.0);
    label
}

fn metric_tile(title: &str, value: &gtk::Label) -> gtk::Box {
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 7);
    tile.add_css_class("iris-metric-card");
    tile.set_hexpand(true);
    tile.set_width_request(150);
    tile.set_margin_top(0);
    tile.set_margin_bottom(0);

    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("iris-field-name");
    title_label.set_xalign(0.0);
    tile.append(&title_label);
    tile.append(value);
    tile
}

fn endpoint_group(title: &str, list: &gtk::ListBox) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("iris-field-name");
    title_label.set_xalign(0.0);
    group.append(&title_label);
    group.append(list);
    group
}

fn close_to_tray_enabled(model: &AppRef) -> bool {
    !model.quit_requested.get() && model.tray_available.get() && model.ui.tray_on_close.is_active()
}

fn add_field(grid: &gtk::Grid, row: i32, column: i32, name: &str, value: &gtk::Label) {
    let label = gtk::Label::new(Some(name));
    label.add_css_class("iris-field-name");
    label.set_xalign(0.0);
    grid.attach(&label, column * 2, row, 1, 1);
    grid.attach(value, column * 2 + 1, row, 1, 1);
}

fn add_copy_field(
    grid: &gtk::Grid,
    row: i32,
    name: &str,
    value: &gtk::Label,
    button: &gtk::Button,
) {
    let label = gtk::Label::new(Some(name));
    label.add_css_class("iris-field-name");
    label.set_xalign(0.0);
    grid.attach(&label, 0, row, 1, 1);
    value.set_hexpand(true);
    grid.attach(value, 1, row, 1, 1);
    grid.attach(button, 2, row, 1, 1);
}

fn connect_tray(model: &AppRef, window: &adw::ApplicationWindow) {
    let (sender, receiver) = mpsc::channel();
    let tray = install_tray(sender);
    let available = tray.is_some();

    model.tray_available.set(available);
    if available && let Some(app) = window.application() {
        TRAY_APP_HOLD.with(|hold| {
            *hold.borrow_mut() = Some(app.hold());
        });
    }
    model.ui.tray_on_close.set_sensitive(available);
    model
        .ui
        .tray_on_close
        .set_active(read_close_to_tray_on_close());
    *model.tray.borrow_mut() = tray;

    if !available {
        return;
    }

    let model = Rc::clone(model);
    let window = window.clone();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if model.retired.get() {
            return glib::ControlFlow::Break;
        }
        while let Ok(command) = receiver.try_recv() {
            handle_tray_command(&model, &window, command);
        }
        glib::ControlFlow::Continue
    });
}

fn handle_tray_command(model: &AppRef, window: &adw::ApplicationWindow, command: TrayCommand) {
    match command {
        TrayCommand::Show => show_window(model, window),
        TrayCommand::OpenDriveFolder => open_drive_folder(model),
        TrayCommand::StartSync => {
            start_daemon(model);
            refresh(model);
        }
        TrayCommand::StopSync => {
            stop_daemon(model);
            refresh(model);
        }
        TrayCommand::Quit => quit_application(model, window),
    }
}

fn show_window(model: &AppRef, window: &adw::ApplicationWindow) {
    if !visible_app_window_exists().unwrap_or_else(|| {
        !model.closed_to_tray.get()
            && window.application().is_some()
            && window.is_mapped()
            && window.is_realized()
            && window.surface().is_some()
    }) {
        model.retired.set(true);
        shutdown_tray(model);
        build_ui(&model.application);
        return;
    }

    window.set_visible(true);
    window.unminimize();
    window.present();
    refresh(model);
}

fn visible_app_window_exists() -> Option<bool> {
    if std::env::var_os("DISPLAY").is_none() {
        return None;
    }

    let status = Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", "^Iris Drive$"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}

fn quit_application(model: &AppRef, window: &adw::ApplicationWindow) {
    model.quit_requested.set(true);
    window.set_hide_on_close(false);
    shutdown_tray(model);
    stop_daemon(model);

    if let Some(app) = window.application() {
        TRAY_APP_HOLD.with(|hold| {
            hold.borrow_mut().take();
        });
        app.quit();
    } else {
        window.close();
    }
}

fn shutdown_tray(model: &AppRef) {
    let tray = model.tray.borrow_mut().take();
    model.tray_available.set(false);
    model.ui.tray_on_close.set_sensitive(false);
    if let Some(tray) = tray {
        shutdown_tray_handle(tray);
    }
}

#[cfg(target_os = "linux")]
fn shutdown_tray_handle(tray: TrayServiceHandle) {
    tray.shutdown().wait();
}

#[cfg(not(target_os = "linux"))]
fn shutdown_tray_handle(_tray: TrayServiceHandle) {}

#[cfg(target_os = "linux")]
fn install_tray(sender: mpsc::Sender<TrayCommand>) -> Option<TrayServiceHandle> {
    use ksni::blocking::TrayMethods;

    let tray = IrisDriveTray { sender };
    match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("Could not start system tray: {error}");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn install_tray(_sender: mpsc::Sender<TrayCommand>) -> Option<TrayServiceHandle> {
    None
}

#[cfg(target_os = "linux")]
struct IrisDriveTray {
    sender: mpsc::Sender<TrayCommand>,
}

#[cfg(target_os = "linux")]
impl IrisDriveTray {
    fn send(&self, command: TrayCommand) {
        let _ = self.sender.send(command);
    }
}

#[cfg(target_os = "linux")]
impl ksni::Tray for IrisDriveTray {
    fn id(&self) -> String {
        APP_ID.to_string()
    }

    fn title(&self) -> String {
        "Iris Drive".to_string()
    }

    fn icon_name(&self) -> String {
        "iris-drive".to_string()
    }

    fn icon_theme_path(&self) -> String {
        if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(path).join("icons").display().to_string();
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".local/share/icons")
                .display()
                .to_string();
        }
        String::new()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: "iris-drive".to_string(),
            title: "Iris Drive".to_string(),
            description: "Sync active".to_string(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            tray_menu_item(
                &self.sender,
                TrayCommand::Show,
                "Show Iris Drive",
                "window-new",
            ),
            tray_menu_item(
                &self.sender,
                TrayCommand::OpenDriveFolder,
                "Open Drive Folder",
                "folder-open",
            ),
            ksni::MenuItem::Separator,
            tray_menu_item(
                &self.sender,
                TrayCommand::StartSync,
                "Start Sync",
                "media-playback-start",
            ),
            tray_menu_item(
                &self.sender,
                TrayCommand::StopSync,
                "Stop Sync",
                "media-playback-stop",
            ),
            ksni::MenuItem::Separator,
            tray_menu_item(&self.sender, TrayCommand::Quit, "Quit", "application-exit"),
        ]
    }
}

#[cfg(target_os = "linux")]
fn tray_menu_item(
    sender: &mpsc::Sender<TrayCommand>,
    command: TrayCommand,
    label: &str,
    icon_name: &str,
) -> ksni::MenuItem<IrisDriveTray> {
    let sender = sender.clone();
    ksni::menu::StandardItem {
        label: label.to_string(),
        icon_name: icon_name.to_string(),
        activate: Box::new(move |_| {
            let _ = sender.send(command);
        }),
        ..Default::default()
    }
    .into()
}

fn refresh(model: &AppRef) {
    match status_with_local_import() {
        Ok((json, scan_notice)) => {
            let initialized = json
                .get("initialized")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let awaiting_link_approval = initialized && is_awaiting_link_approval(&json);
            let sync_running = initialized && ensure_daemon_running(model, &json);
            set_view_mode(model, initialized && !awaiting_link_approval, sync_running);
            if !initialized {
                render_setup(model);
                return;
            }
            if awaiting_link_approval {
                render_awaiting_approval(model, &json, sync_running);
                return;
            }
            model.ui.drive_title.set_text(&drive_name(&json));
            model
                .ui
                .drive_message
                .set_text(if sync_running { "Synced" } else { "Stopped" });
            model
                .ui
                .status_pill
                .set_text(if sync_running { "Running" } else { "Stopped" });
            model
                .ui
                .status
                .set_text(if sync_running { "Syncing" } else { "Ready" });
            model
                .ui
                .folder
                .set_text(&working_dir(&json).display().to_string());
            let account = account_json(&json);
            let owner_npub = find_string(account, &["owner_npub"]);
            let device_npub = find_string(account, &["device_npub"]);
            let authorization = find_string(account, &["authorization_state"]).unwrap_or("-");
            model
                .ui
                .owner
                .set_text(&short_value(owner_npub));
            model
                .ui
                .device
                .set_text(&short_value(device_npub));
            model.ui.account_owner.set_text(owner_npub.unwrap_or("-"));
            model.ui.account_device.set_text(device_npub.unwrap_or("-"));
            model.ui.account_authorization.set_text(authorization);
            model.ui.approve_box.set_visible(
                find_bool(account, &["has_owner_signing_authority"]).unwrap_or(false),
            );
            model.ui.snapshot.set_text(&snapshot_value(&json));
            model.ui.files.set_text(&file_count_value(&json));
            model.ui.blocks.set_text(&block_count_value(&json));
            model.ui.storage.set_text(&storage_value(&json));
            model.ui.devices.set_text(&device_count_value(&json));
            model
                .ui
                .config_path
                .set_text(find_string(&json, &["config_dir"]).unwrap_or("-"));
            model.ui.blocks_path.set_text(
                find_string(
                    json.get("hashtree").unwrap_or(&Value::Null),
                    &["blocks_dir"],
                )
                .unwrap_or("-"),
            );
            model
                .ui
                .drive_path
                .set_text(&working_dir(&json).display().to_string());
            model.ui.root_path.set_text(
                find_string(
                    json.get("hashtree").unwrap_or(&Value::Null),
                    &["current_root_cid"],
                )
                .unwrap_or("-"),
            );
            if let Some(scan_notice) = scan_notice {
                model.ui.notice.set_text(&scan_notice);
            }
            let has_snapshot = snapshot_link(&json).is_some();
            model.ui.copy_snapshot_button.set_sensitive(has_snapshot);
            model.ui.open_snapshot_button.set_sensitive(has_snapshot);
            render_drives(&model.ui.drives, &json);
            render_peers(model, &json);
            render_network(&model.ui.relays, &model.ui.blossom, &json);
        }
        Err(error) => {
            set_view_mode(model, true, daemon_is_running(model));
            model.ui.drive_title.set_text("My Drive");
            model.ui.drive_message.set_text("Unavailable");
            model.ui.status_pill.set_text("Stopped");
            model.ui.status.set_text("Unavailable");
            model
                .ui
                .folder
                .set_text(&default_drive_dir().display().to_string());
            model.ui.owner.set_text("-");
            model.ui.device.set_text("-");
            model.ui.account_owner.set_text("-");
            model.ui.account_device.set_text("-");
            model.ui.account_authorization.set_text("-");
            model.ui.approve_box.set_visible(false);
            model.ui.snapshot.set_text("-");
            model.ui.files.set_text("0");
            model.ui.blocks.set_text("0");
            model.ui.storage.set_text("0 B");
            model.ui.devices.set_text("0/0");
            model.ui.copy_snapshot_button.set_sensitive(false);
            model.ui.open_snapshot_button.set_sensitive(false);
            model.ui.notice.set_text(&error);
            clear_list(&model.ui.drives);
            clear_list(&model.ui.peers);
            clear_list(&model.ui.relays);
            clear_list(&model.ui.blossom);
        }
    }
}

fn status_with_local_import() -> Result<(Value, Option<String>), String> {
    let mut status = run_idrive_json(["status"])?;
    let initialized = status
        .get("initialized")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !initialized {
        return Ok((status, None));
    }
    if is_awaiting_link_approval(&status) {
        return Ok((status, None));
    }

    match import_drive_folder(&working_dir(&status)) {
        Ok(()) => {
            if let Ok(updated) = run_idrive_json(["status"]) {
                status = updated;
            }
            Ok((status, None))
        }
        Err(error) => Ok((
            status,
            Some(format!("Could not scan drive folder: {error}")),
        )),
    }
}

fn set_view_mode(model: &AppRef, initialized: bool, sync_running: bool) {
    model.ui.sidebar.set_visible(initialized);
    model.ui.setup.set_visible(!initialized);
    model.ui.main_view.set_visible(initialized);
    model.ui.main.set_visible(initialized);
    model.ui.init_button.set_visible(false);
    model.ui.folder_button.set_visible(initialized);
    model.ui.copy_snapshot_button.set_visible(initialized);
    model.ui.open_snapshot_button.set_visible(initialized);
    model.ui.restart_button.set_visible(initialized);
    model.ui.restart_button.set_sensitive(initialized);
    model.ui.start_button.set_visible(initialized);
    model
        .ui
        .start_button
        .set_sensitive(initialized && !sync_running);
    model.ui.stop_button.set_visible(initialized);
    model
        .ui
        .stop_button
        .set_sensitive(initialized && sync_running);
}

fn render_setup(model: &AppRef) {
    clear_box(&model.ui.setup);
    match *model.setup_screen.borrow() {
        SetupScreen::Welcome => render_setup_welcome(model),
        SetupScreen::Create => render_create_profile(model),
        SetupScreen::Restore => render_restore_profile(model),
        SetupScreen::Link => render_link_device(model),
    }
}

fn render_awaiting_approval(model: &AppRef, json: &Value, sync_running: bool) {
    clear_box(&model.ui.setup);

    let container = gtk::Box::new(gtk::Orientation::Vertical, 14);
    container.set_halign(gtk::Align::Center);
    container.set_valign(gtk::Align::Center);
    container.set_width_request(420);

    let header = gtk::Label::new(Some("Awaiting approval"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let account = account_json(json);
    let owner = readonly_entry(find_string(account, &["owner_npub"]).unwrap_or("-"));
    container.append(&field_title("Owner"));
    container.append(&owner);

    let device = readonly_entry(find_string(account, &["device_npub"]).unwrap_or("-"));
    container.append(&field_title("This device"));
    container.append(&device);

    let request = find_string(
        account
            .get("device_link_request")
            .unwrap_or(&Value::Null),
        &["url"],
    )
    .unwrap_or("");
    let request_entry = readonly_entry(request);
    request_entry.set_height_request(82);
    container.append(&field_title("Request link"));
    container.append(&request_entry);

    let notice = setup_notice();
    notice.set_text(if sync_running {
        "Waiting for approval"
    } else {
        "Sync stopped"
    });

    let copy = primary_button("Copy request");
    {
        let request = request.to_string();
        let notice = notice.clone();
        copy.connect_clicked(move |_| {
            if request.is_empty() {
                notice.set_text("Nothing to copy");
            } else if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&request);
                notice.set_text("Request copied");
            } else {
                notice.set_text("Clipboard unavailable");
            }
        });
    }
    container.append(&copy);
    container.append(&notice);

    append_centered_setup(model, &container);
}

fn setup_container(model: &AppRef, title: &str) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 14);
    container.set_halign(gtk::Align::Center);
    container.set_valign(gtk::Align::Center);
    container.set_width_request(340);

    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some("Back"));
    back.set_halign(gtk::Align::Start);
    {
        let model = Rc::clone(model);
        back.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
            render_setup(&model);
        });
    }
    container.append(&back);

    let header = gtk::Label::new(Some(title));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    container
}

fn render_setup_welcome(model: &AppRef) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 18);
    container.set_halign(gtk::Align::Center);
    container.set_width_request(340);

    let icon = gtk::Image::from_icon_name("iris-drive");
    icon.set_pixel_size(96);
    icon.set_margin_bottom(4);
    container.append(&icon);

    let title = gtk::Label::new(Some("Iris Drive"));
    title.add_css_class("title-1");
    title.set_halign(gtk::Align::Center);
    title.set_margin_bottom(10);
    container.append(&title);

    let create = welcome_button("Create profile", "list-add-symbolic", true);
    {
        let model = Rc::clone(model);
        create.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::Create;
            render_setup(&model);
        });
    }
    container.append(&create);

    let restore = welcome_button("Restore profile", "dialog-password-symbolic", false);
    {
        let model = Rc::clone(model);
        restore.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::Restore;
            render_setup(&model);
        });
    }
    container.append(&restore);

    let link = welcome_button("Link this device", "computer-symbolic", false);
    {
        let model = Rc::clone(model);
        link.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::Link;
            render_setup(&model);
        });
    }
    container.append(&link);

    append_centered_setup(model, &container);
}

fn welcome_button(label: &str, icon_name: &str, primary: bool) -> gtk::Button {
    let button = if primary {
        primary_button(label)
    } else {
        pill_button(label)
    };
    button.set_width_request(340);

    let content = adw::ButtonContent::builder()
        .icon_name(icon_name)
        .label(label)
        .build();
    button.set_child(Some(&content));
    button
}

fn render_create_profile(model: &AppRef) {
    let container = setup_container(model, "Create profile");
    let label = setup_entry("Device label");
    container.append(&label);

    let notice = setup_notice();
    let submit = primary_button("Create profile");
    {
        let model = Rc::clone(model);
        let label = label.clone();
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            button.set_sensitive(false);
            match create_profile(label.text().trim()) {
                Ok(()) => {
                    *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
                    refresh(&model);
                }
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&submit);
    container.append(&notice);
    append_centered_setup(model, &container);

    label.grab_focus();
}

fn render_restore_profile(model: &AppRef) {
    let container = setup_container(model, "Restore profile");
    let nsec = setup_entry("Secret key");
    nsec.set_visibility(false);
    nsec.set_input_purpose(gtk::InputPurpose::Password);
    container.append(&nsec);

    let label = setup_entry("Device label");
    container.append(&label);

    let notice = setup_notice();
    let submit = primary_button("Restore profile");
    {
        let model = Rc::clone(model);
        let nsec = nsec.clone();
        let label = label.clone();
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            let secret = nsec.text().trim().to_string();
            if secret.is_empty() {
                notice.set_text("Secret key is required.");
                return;
            }
            button.set_sensitive(false);
            match restore_profile(&secret, label.text().trim()) {
                Ok(()) => {
                    *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
                    refresh(&model);
                }
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&submit);
    container.append(&notice);
    append_centered_setup(model, &container);

    nsec.grab_focus();
}

fn render_link_device(model: &AppRef) {
    let container = setup_container(model, "Link this device");
    let owner = setup_entry("Owner public key");
    container.append(&owner);

    let label = setup_entry("Device label");
    container.append(&label);

    let notice = setup_notice();
    let submit = primary_button("Link device");
    {
        let model = Rc::clone(model);
        let owner = owner.clone();
        let label = label.clone();
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            let owner_value = owner.text().trim().to_string();
            if owner_value.is_empty() {
                notice.set_text("Owner public key is required.");
                return;
            }
            button.set_sensitive(false);
            match link_device(&owner_value, label.text().trim()) {
                Ok(()) => {
                    *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
                    refresh(&model);
                }
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&submit);
    container.append(&notice);
    append_centered_setup(model, &container);

    owner.grab_focus();
}

fn append_centered_setup(model: &AppRef, child: &gtk::Box) {
    let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top.set_vexpand(true);
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.set_vexpand(true);
    model.ui.setup.append(&top);
    model.ui.setup.append(child);
    model.ui.setup.append(&bottom);
}

fn setup_notice() -> gtk::Label {
    let notice = gtk::Label::new(None);
    notice.add_css_class("iris-notice");
    notice.set_xalign(0.0);
    notice.set_wrap(true);
    notice
}

fn initialize_drive(model: &AppRef) {
    match create_profile("") {
        Ok(()) => {
            model.ui.notice.set_text("Initialized");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

fn create_profile(label: &str) -> Result<(), String> {
    let mut init_args = vec!["init".to_string(), "--force".to_string()];
    let label = label.trim();
    if !label.is_empty() {
        init_args.push("--label".to_string());
        init_args.push(label.to_string());
    }

    run_idrive_owned(&init_args)?;
    import_default_drive()
}

fn restore_profile(secret: &str, label: &str) -> Result<(), String> {
    let mut args = vec!["restore".to_string(), secret.to_string()];
    let label = label.trim();
    if !label.is_empty() {
        args.push("--label".to_string());
        args.push(label.to_string());
    }
    run_idrive_owned(&args)?;
    import_default_drive()
}

fn link_device(owner: &str, label: &str) -> Result<(), String> {
    let mut args = vec!["link".to_string(), owner.to_string()];
    let label = label.trim();
    if !label.is_empty() {
        args.push("--label".to_string());
        args.push(label.to_string());
    }
    run_idrive_owned(&args)?;
    import_default_drive()
}

fn revoke_device(device: &str) -> Result<(), String> {
    run_idrive(["revoke", device])
}

fn import_default_drive() -> Result<(), String> {
    import_drive_folder(&default_drive_dir())
}

fn import_drive_folder(folder: &PathBuf) -> Result<(), String> {
    if let Err(error) = std::fs::create_dir_all(folder) {
        return Err(format!("Could not create drive folder: {error}"));
    }

    let folder_arg = folder.display().to_string();
    run_idrive(["import", folder_arg.as_str()])
}

fn start_daemon(model: &AppRef) {
    let status = run_idrive_json(["status"]).unwrap_or(Value::Null);
    if ensure_daemon_running(model, &status) {
        model.ui.notice.set_text("Sync already running");
        return;
    }
    model.ui.notice.set_text("Could not start sync");
}

fn restart_daemon(model: &AppRef) {
    stop_daemon(model);
    start_daemon(model);
    refresh(model);
}

fn ensure_daemon_running(model: &AppRef, status: &Value) -> bool {
    if daemon_is_running(model) || daemon_lock_is_running(status) {
        return true;
    }

    match spawn_daemon() {
        Ok(child) => {
            *model.daemon.borrow_mut() = Some(child);
            model.ui.notice.set_text("Sync started");
            true
        }
        Err(error) => {
            model
                .ui
                .notice
                .set_text(&format!("Could not start sync: {error}"));
            false
        }
    }
}

fn spawn_daemon() -> Result<Child, std::io::Error> {
    match Command::new(idrive_path())
        .arg("daemon")
        .arg("--watch-interval")
        .arg("10")
        .env("IRIS_DRIVE_PARENT_PID", std::process::id().to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => Ok(child),
        Err(error) => Err(error),
    }
}

fn daemon_is_running(model: &AppRef) -> bool {
    let mut daemon = model.daemon.borrow_mut();
    let Some(child) = daemon.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(None) => true,
        Ok(Some(_)) | Err(_) => {
            *daemon = None;
            false
        }
    }
}

fn daemon_lock_is_running(status: &Value) -> bool {
    let Some(config_dir) = status.get("config_dir").and_then(Value::as_str) else {
        return false;
    };
    let Ok(contents) = std::fs::read_to_string(PathBuf::from(config_dir).join("daemon.lock"))
    else {
        return false;
    };
    let Ok(pid) = contents.trim().parse::<u32>() else {
        return false;
    };
    process_is_running(pid)
}

fn process_is_running(pid: u32) -> bool {
    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

struct AppInstanceLock {
    path: PathBuf,
}

impl AppInstanceLock {
    fn acquire() -> Result<Self, String> {
        let dir = app_config_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("Could not create config dir {}: {error}", dir.display()))?;
        let path = dir.join("app.lock");

        match Self::try_create(&path) {
            Ok(lock) => Ok(lock),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Self::replace_stale_or_error(&path)
            }
            Err(error) => Err(format!(
                "Could not create app lock {}: {error}",
                path.display()
            )),
        }
    }

    fn replace_stale_or_error(path: &Path) -> Result<Self, String> {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<u32>()
            && process_is_running(pid)
        {
            return Err("Iris Drive is already running.".to_string());
        }

        let _ = std::fs::remove_file(path);
        Self::try_create(path).map_err(|error| {
            format!(
                "Could not replace stale app lock {}: {error}",
                path.display()
            )
        })
    }

    fn try_create(path: &Path) -> std::io::Result<Self> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for AppInstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn app_config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("IRIS_DRIVE_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("iris-drive");
    }
    if let Some(path) = std::env::var_os("HOME") {
        return PathBuf::from(path).join(".config/iris-drive");
    }
    PathBuf::from(".").join(".config/iris-drive")
}

fn close_to_tray_config_path() -> PathBuf {
    app_config_dir().join("linux-close-to-tray-on-close")
}

fn read_close_to_tray_on_close() -> bool {
    std::fs::read_to_string(close_to_tray_config_path())
        .map(|value| value.trim() != "false")
        .unwrap_or(true)
}

fn write_close_to_tray_on_close(enabled: bool) {
    let path = close_to_tray_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, if enabled { "true\n" } else { "false\n" });
}

fn stop_daemon(model: &AppRef) {
    let Some(mut child) = model.daemon.borrow_mut().take() else {
        return;
    };
    let _ = child.kill();
    let _ = child.wait();
    model.ui.notice.set_text("Sync stopped");
    refresh(model);
}

fn add_relay(model: &AppRef) {
    let relay = model.ui.relay_entry.text().trim().to_string();
    if relay.is_empty() {
        return;
    }
    match run_idrive_owned(&["relays".to_string(), "add".to_string(), relay]) {
        Ok(()) => {
            model.ui.relay_entry.set_text("");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

fn reset_relays(model: &AppRef) {
    match run_idrive(["relays", "reset"]) {
        Ok(()) => refresh(model),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

fn approve_device(model: &AppRef) {
    let device = model.ui.approve_device_entry.text().trim().to_string();
    if device.is_empty() {
        model.ui.notice.set_text("Device key is required");
        return;
    }

    let mut args = vec!["approve".to_string(), device];
    let label = model.ui.approve_label_entry.text().trim().to_string();
    if !label.is_empty() {
        args.push("--label".to_string());
        args.push(label);
    }

    match run_idrive_owned(&args) {
        Ok(()) => {
            model.ui.approve_device_entry.set_text("");
            model.ui.approve_label_entry.set_text("");
            restart_daemon(model);
            model.ui.notice.set_text("Device approved");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

fn open_drive_folder(model: &AppRef) {
    let folder = run_idrive_json(["status"])
        .map(|json| working_dir(&json))
        .unwrap_or_else(|_| default_drive_dir());
    if let Err(error) = std::fs::create_dir_all(&folder) {
        model
            .ui
            .notice
            .set_text(&format!("Could not create drive folder: {error}"));
        return;
    }
    open_path(&folder);
}

fn copy_snapshot_link(model: &AppRef) {
    match current_snapshot_link() {
        Ok(link) => copy_text(model, &link, "Snapshot copied"),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

fn copy_account_key(model: &AppRef, key: &str) {
    match current_account_value(key) {
        Ok(value) => {
            let message = if key == "owner_npub" {
                "Owner key copied"
            } else {
                "Device key copied"
            };
            copy_text(model, &value, message);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

fn copy_text(model: &AppRef, value: &str, message: &str) {
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(value);
        model.ui.notice.set_text(message);
    } else {
        model.ui.notice.set_text("Clipboard unavailable");
    }
}

fn open_snapshot_link(model: &AppRef) {
    match current_snapshot_link() {
        Ok(link) => open_uri(&link),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

fn current_snapshot_link() -> Result<String, String> {
    let json = run_idrive_json(["status"])?;
    snapshot_link(&json)
        .map(str::to_string)
        .ok_or_else(|| "No snapshot available".to_string())
}

fn current_account_value(key: &str) -> Result<String, String> {
    let json = run_idrive_json(["status"])?;
    let account = account_json(&json);
    find_string(account, &[key])
        .map(str::to_string)
        .ok_or_else(|| "No account key available".to_string())
}

fn run_idrive_json<const N: usize>(args: [&str; N]) -> Result<Value, String> {
    let output = Command::new(idrive_path())
        .args(args)
        .output()
        .map_err(|error| format!("idrive failed to start: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("Invalid status JSON: {error}"))
}

fn run_idrive<const N: usize>(args: [&str; N]) -> Result<(), String> {
    let output = Command::new(idrive_path())
        .args(args)
        .output()
        .map_err(|error| format!("idrive failed to start: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn run_idrive_owned(args: &[String]) -> Result<(), String> {
    let output = Command::new(idrive_path())
        .args(args)
        .output()
        .map_err(|error| format!("idrive failed to start: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn idrive_path() -> PathBuf {
    if let Ok(path) = std::env::var("IRIS_DRIVE_CLI") {
        return PathBuf::from(path);
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for candidate in [
        manifest.join("../target/debug/idrive"),
        manifest.join("../target/release/idrive"),
        manifest.join("../../target/debug/idrive"),
        manifest.join("../../target/release/idrive"),
    ] {
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("idrive")
}

fn default_drive_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Iris Drive")
}

fn working_dir(json: &Value) -> PathBuf {
    if let Some(path) = json
        .get("drives")
        .and_then(Value::as_array)
        .and_then(|drives| {
            drives
                .iter()
                .find(|drive| drive.get("drive_id").and_then(Value::as_str) == Some("main"))
        })
        .and_then(|drive| drive.get("working_dir"))
        .and_then(Value::as_str)
    {
        return PathBuf::from(path);
    }
    if let Some(path) = json.get("default_working_dir").and_then(Value::as_str) {
        return PathBuf::from(path);
    }
    default_drive_dir()
}

fn drive_name(json: &Value) -> String {
    json.get("drives")
        .and_then(Value::as_array)
        .and_then(|drives| {
            drives
                .iter()
                .find(|drive| drive.get("drive_id").and_then(Value::as_str) == Some("main"))
                .or_else(|| drives.first())
        })
        .and_then(|drive| find_string(drive, &["display_name", "name"]))
        .unwrap_or("My Drive")
        .to_string()
}

fn snapshot_value(json: &Value) -> String {
    snapshot_link(json)
        .map(short_text)
        .unwrap_or_else(|| "-".to_string())
}

fn snapshot_link(json: &Value) -> Option<&str> {
    let hashtree = json.get("hashtree").unwrap_or(&Value::Null);
    find_string(hashtree, &["snapshot_url", "permalink_url"])
}

fn account_json(json: &Value) -> &Value {
    json.get("account").unwrap_or(&Value::Null)
}

fn is_awaiting_link_approval(json: &Value) -> bool {
    let account = account_json(json);
    find_string(account, &["authorization_state"]) == Some("awaiting_approval")
        && find_bool(account, &["has_owner_signing_authority"]) == Some(false)
}

fn file_count_value(json: &Value) -> String {
    let hashtree = json.get("hashtree").unwrap_or(&Value::Null);
    find_number(hashtree, &["file_count", "top_level_entries"])
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_string())
}

fn block_count_value(json: &Value) -> String {
    let hashtree = json.get("hashtree").unwrap_or(&Value::Null);
    find_number(hashtree, &["local_block_count"])
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_string())
}

fn storage_value(json: &Value) -> String {
    let hashtree = json.get("hashtree").unwrap_or(&Value::Null);
    find_number(hashtree, &["local_block_bytes"])
        .map(format_bytes)
        .unwrap_or_else(|| "0 B".to_string())
}

fn device_count_value(json: &Value) -> String {
    let network = json.get("network").unwrap_or(&Value::Null);
    let published = find_number(network, &["published_device_roots"]).unwrap_or(0);
    let authorized = find_number(network, &["authorized_device_count"]).unwrap_or(0);
    format!("{published}/{authorized}")
}

fn render_drives(list: &gtk::ListBox, json: &Value) {
    clear_list(list);
    let Some(drives) = json.get("drives").and_then(Value::as_array) else {
        list.append(&drive_row(
            "main",
            &working_dir(json).display().to_string(),
            "-",
        ));
        return;
    };
    if drives.is_empty() {
        list.append(&drive_row(
            "main",
            &working_dir(json).display().to_string(),
            "-",
        ));
        return;
    }
    for drive in drives {
        let name = find_string(drive, &["name", "drive_id"]).unwrap_or("main");
        let path = find_string(drive, &["working_dir", "local_path"]).unwrap_or("-");
        let status = find_string(drive, &["status", "root_cid"])
            .map(short_text)
            .unwrap_or_else(|| "configured".to_string());
        list.append(&drive_row(name, path, &status));
    }
}

fn render_peers(model: &AppRef, json: &Value) {
    let list = &model.ui.peers;
    clear_list(list);
    let Some(peers) = json.get("peers").and_then(Value::as_array) else {
        list.append(&simple_row("No authorized devices", ""));
        return;
    };
    if peers.is_empty() {
        list.append(&simple_row("No authorized devices", ""));
        return;
    }
    let can_manage_devices = json
        .get("account")
        .and_then(|account| account.get("has_owner_signing_authority"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for peer in peers {
        let title =
            find_string(peer, &["label", "device_npub", "device_pubkey"]).unwrap_or("Device");
        let device_npub = find_string(peer, &["device_npub"]).unwrap_or("");
        let is_current_device = peer
            .get("is_current_device")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut metadata = Vec::new();
        if is_current_device {
            metadata.push("this device".to_string());
        }
        if let Some(sync_state) = find_string(peer, &["sync_state"]) {
            metadata.push(sync_state.to_string());
        }
        if let Some(last_sync) = peer.get("last_block_sync") {
            if let (Some(transport), Some(total)) = (
                find_string(last_sync, &["transport"]),
                find_number(last_sync, &["total_hashes"]),
            ) {
                let fetched = find_number(last_sync, &["fetched"]).unwrap_or(0);
                metadata.push(format!("{transport} {fetched}/{total}"));
            }
        }
        if let Some(root) = find_string(peer, &["root_cid"]) {
            metadata.push(short_text(root));
        }
        if let Some(generation) = find_number(peer, &["dck_generation"]) {
            metadata.push(format!("DCK {generation}"));
        }
        let state = if peer
            .get("fips_online")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "Online"
        } else {
            "Offline"
        };
        list.append(&peer_row(
            model,
            title,
            &metadata.join(" | "),
            state,
            device_npub,
            can_manage_devices && !is_current_device && !device_npub.is_empty(),
        ));
    }
}

fn render_network(relays_list: &gtk::ListBox, blossom_list: &gtk::ListBox, json: &Value) {
    clear_list(relays_list);
    clear_list(blossom_list);
    let network = json.get("network").unwrap_or(&Value::Null);

    if let Some(relays) = network.get("relays").and_then(Value::as_array) {
        for relay in relays.iter().filter_map(Value::as_str) {
            relays_list.append(&simple_row(relay, relay_status(relay, network)));
        }
    }
    if relays_list.first_child().is_none() {
        relays_list.append(&simple_row("No relays", ""));
    }

    if let Some(servers) = network.get("blossom_servers").and_then(Value::as_array) {
        for server in servers.iter().filter_map(Value::as_str) {
            blossom_list.append(&simple_row(server, ""));
        }
    }
    if blossom_list.first_child().is_none() {
        blossom_list.append(&simple_row("No Blossom servers", ""));
    }
}

fn relay_status<'a>(relay: &str, network: &'a Value) -> &'a str {
    network
        .get("relay_statuses")
        .and_then(Value::as_array)
        .and_then(|statuses| {
            statuses.iter().find_map(|status| {
                let url = status.get("url").and_then(Value::as_str)?;
                if url == relay {
                    status.get("status").and_then(Value::as_str)
                } else {
                    None
                }
            })
        })
        .unwrap_or("saved")
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn simple_row(title: &str, subtitle: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 3);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(12);
    body.set_margin_end(12);

    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("iris-row-title");
    title_label.set_xalign(0.0);
    title_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title_label.set_max_width_chars(44);
    body.append(&title_label);

    if !subtitle.is_empty() {
        let subtitle_label = gtk::Label::new(Some(subtitle));
        subtitle_label.add_css_class("iris-row-subtitle");
        subtitle_label.set_xalign(0.0);
        subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        subtitle_label.set_max_width_chars(44);
        body.append(&subtitle_label);
    }

    row.set_child(Some(&body));
    row
}

fn peer_row(
    model: &AppRef,
    title: &str,
    subtitle: &str,
    state: &str,
    device_npub: &str,
    can_revoke: bool,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(12);
    body.set_margin_end(12);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("iris-row-title");
    title_label.set_xalign(0.0);
    title_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title_label.set_max_width_chars(44);
    labels.append(&title_label);

    if !subtitle.is_empty() {
        let subtitle_label = gtk::Label::new(Some(subtitle));
        subtitle_label.add_css_class("iris-row-subtitle");
        subtitle_label.set_xalign(0.0);
        subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        subtitle_label.set_max_width_chars(44);
        labels.append(&subtitle_label);
    }
    body.append(&labels);

    let state_label = gtk::Label::new(Some(state));
    state_label.add_css_class("iris-row-state");
    state_label.set_xalign(1.0);
    body.append(&state_label);

    if can_revoke {
        let revoke = icon_button("user-trash-symbolic", "Revoke device");
        let model = Rc::clone(model);
        let device_npub = device_npub.to_string();
        revoke.connect_clicked(move |_| match revoke_device(&device_npub) {
            Ok(()) => {
                restart_daemon(&model);
                model.ui.notice.set_text("Device revoked");
                refresh(&model);
            }
            Err(error) => model.ui.notice.set_text(&error),
        });
        body.append(&revoke);
    }

    row.set_child(Some(&body));
    row
}

fn drive_row(name: &str, path: &str, status: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(12);
    body.set_margin_end(12);

    let icon = gtk::Image::from_icon_name("folder-symbolic");
    body.append(&icon);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(name));
    title.add_css_class("iris-row-title");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(Some(path));
    subtitle.add_css_class("iris-row-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_hexpand(true);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    subtitle.set_max_width_chars(44);
    labels.append(&title);
    labels.append(&subtitle);
    body.append(&labels);

    let state = gtk::Label::new(Some(status));
    state.add_css_class("iris-row-state");
    state.set_xalign(1.0);
    body.append(&state);

    row.set_child(Some(&body));
    row
}

fn find_string<'a>(json: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| json.get(*key).and_then(Value::as_str))
}

fn find_number(json: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| json.get(*key).and_then(Value::as_u64))
}

fn find_bool(json: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| json.get(*key).and_then(Value::as_bool))
}

fn short_value(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "-".to_string();
    };
    short_text(value)
}

fn short_text(value: &str) -> String {
    if value.chars().count() <= 32 {
        return value.to_string();
    }
    let start = value.chars().take(14).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(10)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{start}...{end}")
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn open_path(path: &PathBuf) {
    let _ = Command::new("xdg-open").arg(path).spawn();
}

fn open_uri(uri: &str) {
    let _ = gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>);
}

fn install_css() {
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
