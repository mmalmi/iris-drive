#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn build_ui(app: &adw::Application) {
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

    let folder_button = action_button(
        "folder-open-symbolic",
        "Open Drive Folder",
        "Open drive folder",
    );
    let copy_snapshot_button = action_button(
        "insert-link-symbolic",
        "Copy Snapshot",
        "Copy snapshot link",
    );
    let open_snapshot_button =
        action_button("document-open-symbolic", "Open Snapshot", "Open snapshot");
    let init_button = text_button("Initialize");
    let stop_button = action_button("media-playback-pause-symbolic", "Pause", "Pause sync");
    let start_button = action_button("media-playback-start-symbolic", "Resume", "Resume sync");

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
    let drive_message = gtk::Label::new(Some("Turning sync on"));
    drive_message.add_css_class("iris-muted");
    drive_message.set_xalign(0.0);
    drive_labels.append(&drive_title);
    drive_labels.append(&drive_message);
    drive_header.append(&drive_labels);

    let status_pill = gtk::Label::new(Some("Paused"));
    status_pill.add_css_class("iris-status-pill");
    drive_header.append(&status_pill);
    dashboard.append(&drive_header);

    let status = value_label();
    let folder = value_label();
    let owner = value_label();
    let device = value_label();
    let snapshot = value_label();
    let files = metric_value_label();
    let storage = metric_value_label();
    let devices = metric_value_label();

    let metrics = flow_section("iris-metrics", 1, 4);
    metrics.append(&metric_tile("Files", &files));
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
    add_copy_field(
        &account_grid,
        0,
        "Owner",
        &account_owner,
        &copy_owner_button,
    );
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

    let backups_page = page_box();
    backups_page.append(&section_title("Backups"));
    let backups = gtk::ListBox::new();
    backups.add_css_class("iris-drive-list");
    backups.set_selection_mode(gtk::SelectionMode::None);
    backups_page.append(&backups);
    let backup_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let backup_entry = setup_entry("https://, npub1..., fs:/path, lmdb:/path");
    backup_entry.set_hexpand(true);
    let backup_label_entry = setup_entry("Label");
    backup_label_entry.set_width_request(140);
    let add_backup_button = icon_button("list-add-symbolic", "Add backup");
    let check_backups_button =
        action_button("emblem-default-symbolic", "Check", "Check backups");
    let sync_backups_button =
        action_button("emblem-synchronizing-symbolic", "Sync", "Sync backups");
    backup_controls.append(&backup_entry);
    backup_controls.append(&backup_label_entry);
    backup_controls.append(&add_backup_button);
    backup_controls.append(&check_backups_button);
    backup_controls.append(&sync_backups_button);
    backups_page.append(&backup_controls);

    let network_page = page_box();
    network_page.append(&section_title("Network"));
    let fips = gtk::ListBox::new();
    fips.add_css_class("iris-drive-list");
    fips.set_selection_mode(gtk::SelectionMode::None);
    network_page.append(&endpoint_group("FIPS", &fips));

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

    let settings_page = page_box();
    settings_page.append(&section_title("Settings"));
    let tray_on_close = gtk::CheckButton::with_label("Tray on close");
    tray_on_close.add_css_class("iris-setting-check");
    tray_on_close.set_active(read_close_to_tray_on_close());
    tray_on_close.set_sensitive(false);
    settings_page.append(&tray_on_close);
    let local_nhash_resolver = gtk::CheckButton::with_label("nhash.iris.localhost resolver");
    local_nhash_resolver.add_css_class("iris-setting-check");
    local_nhash_resolver.set_active(true);
    settings_page.append(&local_nhash_resolver);

    stack.add_titled(&dashboard, Some("drive"), "My Drive");
    stack.add_titled(&peers_page, Some("devices"), "Devices");
    stack.add_titled(&backups_page, Some("backups"), "Backups");
    stack.add_titled(&network_page, Some("network"), "Network");
    stack.add_titled(&settings_page, Some("settings"), "Settings");

    let nav_items = [
        ("drive", "drive-harddisk-symbolic", "My Drive"),
        ("devices", "system-users-symbolic", "Devices"),
        ("backups", "security-high-symbolic", "Backups"),
        ("network", "network-workgroup-symbolic", "Network"),
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

    sidebar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let sidebar_summary = gtk::Box::new(gtk::Orientation::Vertical, 4);
    sidebar_summary.add_css_class("iris-sidebar-summary");
    let sidebar_online = gtk::Label::new(Some("0/0 online"));
    sidebar_online.add_css_class("caption");
    sidebar_online.add_css_class("dim-label");
    sidebar_online.set_xalign(0.0);
    sidebar_summary.append(&sidebar_online);
    sidebar.append(&sidebar_summary);

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
            sidebar_online,
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
            backups,
            fips,
            relays,
            blossom,
            tray_on_close,
            local_nhash_resolver,
            relay_entry,
            backup_entry,
            backup_label_entry,
            init_button,
            folder_button,
            copy_snapshot_button,
            open_snapshot_button,
            start_button,
            stop_button,
        },
        daemon: RefCell::new(None),
        setup_screen: RefCell::new(SetupScreen::Welcome),
        setup_username: RefCell::new(String::new()),
        tray: RefCell::new(None),
        tray_available: Cell::new(false),
        settings_refreshing: Cell::new(false),
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
    {
        let model = Rc::clone(&model);
        add_backup_button.connect_clicked(move |_| add_backup_target(&model));
    }
    {
        let model = Rc::clone(&model);
        check_backups_button.connect_clicked(move |_| check_backups(&model));
    }
    {
        let model = Rc::clone(&model);
        sync_backups_button.connect_clicked(move |_| sync_backups(&model));
    }
    model.ui.tray_on_close.connect_toggled(|button| {
        write_close_to_tray_on_close(button.is_active());
    });
    {
        let button = model.ui.local_nhash_resolver.clone();
        let model = Rc::clone(&model);
        button.connect_toggled(move |button| {
            if model.settings_refreshing.get() {
                return;
            }
            set_local_nhash_resolver(&model, button.is_active());
        });
    }

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
