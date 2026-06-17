#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn build_ui(app: &adw::Application, present: bool) {
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

    let update_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    update_bar.add_css_class("iris-update-bar");
    update_bar.set_margin_top(8);
    update_bar.set_margin_bottom(8);
    update_bar.set_margin_start(18);
    update_bar.set_margin_end(18);
    update_bar.set_visible(false);
    let update_label = gtk::Label::new(Some(""));
    update_label.set_xalign(0.0);
    update_label.set_hexpand(true);
    let update_auto_check = gtk::CheckButton::with_label("Check automatically");
    let update_auto_install = gtk::CheckButton::with_label("Install automatically");
    let update_install_button =
        action_button("folder-download-symbolic", "Download", "Download update");
    update_bar.append(&update_label);
    update_bar.append(&update_install_button);
    root.append(&update_bar);

    let folder_button = action_button(
        "folder-open-symbolic",
        "Open Drive Folder",
        "Open drive folder",
    );
    let copy_snapshot_button = action_button(
        "insert-link-symbolic",
        "Copy Link",
        "Copy drive.iris.to link",
    );
    let open_snapshot_button = action_button(
        "document-open-symbolic",
        "View on drive.iris.to",
        "View on drive.iris.to",
    );
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
    let app_key = value_label();
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
    let account_app_key = value_label();
    let account_device = value_label();
    let account_authorization = value_label();
    let copy_app_key_button = icon_button("edit-copy-symbolic", "Copy device key");
    let copy_device_button = icon_button("edit-copy-symbolic", "Copy device key");
    let account_grid = gtk::Grid::new();
    account_grid.add_css_class("iris-summary");
    account_grid.set_column_spacing(10);
    account_grid.set_row_spacing(8);
    account_grid.set_hexpand(true);
    add_copy_field(
        &account_grid,
        0,
        "Device",
        &account_app_key,
        &copy_app_key_button,
    );
    add_copy_field(
        &account_grid,
        1,
        "Current Device Key",
        &account_device,
        &copy_device_button,
    );
    peers_page.append(&account_grid);

    let approve_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    approve_box.set_hexpand(true);
    let approve_device_button = action_button("list-add-symbolic", "Add Device", "Add device");
    let add_recovery_key_button = action_button(
        "dialog-password-symbolic",
        "Add Recovery Key",
        "Add recovery key",
    );
    let reset_invite_button =
        action_button("view-refresh-symbolic", "Reset invite", "Reset invite");
    approve_box.append(&approve_device_button);
    approve_box.append(&add_recovery_key_button);
    approve_box.append(&reset_invite_button);
    peers_page.append(&approve_box);

    peers_page.append(&field_title("Linked Devices"));
    let peers = gtk::ListBox::new();
    peers.add_css_class("iris-drive-list");
    peers.set_selection_mode(gtk::SelectionMode::None);
    peers_page.append(&peers);

    let backups_page = page_box();
    backups_page.append(&section_title("Backup"));
    let backups = gtk::ListBox::new();
    backups.add_css_class("iris-drive-list");
    backups.set_selection_mode(gtk::SelectionMode::None);
    backups_page.append(&backups);
    let backup_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let backup_entry = setup_entry("Destination URL, User ID, or folder path");
    backup_entry.set_hexpand(true);
    let backup_label_entry = setup_entry("Label");
    backup_label_entry.set_width_request(140);
    let add_backup_button = icon_button("list-add-symbolic", "Add Backup");
    let check_backups_button = action_button("emblem-default-symbolic", "Check", "Check backups");
    let sync_backups_button =
        action_button("emblem-synchronizing-symbolic", "Sync", "Sync backups");
    backup_controls.append(&backup_entry);
    backup_controls.append(&backup_label_entry);
    backup_controls.append(&add_backup_button);
    backup_controls.append(&check_backups_button);
    backup_controls.append(&sync_backups_button);
    backups_page.append(&backup_controls);

    let shares_page = page_box();
    shares_page.append(&section_title("Shares"));
    let shares = gtk::ListBox::new();
    shares.add_css_class("iris-drive-list");
    shares.set_selection_mode(gtk::SelectionMode::None);
    shares_page.append(&shares);

    let share_create_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let share_source_entry = setup_entry("Folder path");
    share_source_entry.set_hexpand(true);
    let create_share_button =
        action_button("folder-new-symbolic", "Create", "Create shared folder");
    share_create_controls.append(&share_source_entry);
    share_create_controls.append(&create_share_button);
    shares_page.append(&share_create_controls);

    let share_accept_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let share_invite_entry = setup_entry("Share invite");
    share_invite_entry.set_hexpand(true);
    let accept_share_invite_button =
        action_button("mail-mark-read-symbolic", "Accept", "Accept share invite");
    share_accept_controls.append(&share_invite_entry);
    share_accept_controls.append(&accept_share_invite_button);
    shares_page.append(&share_accept_controls);

    let last_invite_grid = gtk::Grid::new();
    last_invite_grid.add_css_class("iris-summary");
    last_invite_grid.set_column_spacing(10);
    last_invite_grid.set_row_spacing(8);
    last_invite_grid.set_hexpand(true);
    let last_share_invite = value_label();
    let copy_last_share_invite_button = icon_button("edit-copy-symbolic", "Copy invite");
    let share_identity_placeholder = value_label();
    share_identity_placeholder.set_text("Signed IrisProfile proof");
    let copy_share_identity_button = icon_button("edit-copy-symbolic", "Copy my share identity");
    add_copy_field(
        &last_invite_grid,
        0,
        "Last invite",
        &last_share_invite,
        &copy_last_share_invite_button,
    );
    add_copy_field(
        &last_invite_grid,
        1,
        "My share identity",
        &share_identity_placeholder,
        &copy_share_identity_button,
    );
    shares_page.append(&last_invite_grid);

    let network_page = page_box();
    network_page.append(&section_title("Network"));
    let fips = gtk::ListBox::new();
    fips.add_css_class("iris-drive-list");
    fips.set_selection_mode(gtk::SelectionMode::None);
    network_page.append(&endpoint_group("FIPS", &fips));

    let blossom = gtk::ListBox::new();
    blossom.add_css_class("iris-drive-list");
    blossom.set_selection_mode(gtk::SelectionMode::None);
    network_page.append(&endpoint_group("File Servers", &blossom));

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
    let launch_on_startup = gtk::CheckButton::with_label("Launch on startup");
    launch_on_startup.add_css_class("iris-setting-check");
    launch_on_startup.set_active(true);
    settings_page.append(&launch_on_startup);
    let local_nhash_resolver = gtk::CheckButton::with_label("*.iris.localhost resolver");
    local_nhash_resolver.add_css_class("iris-setting-check");
    local_nhash_resolver.set_active(true);
    settings_page.append(&local_nhash_resolver);
    let open_sites_portal_button = action_button(
        "web-browser-symbolic",
        "Open Iris Apps",
        "Open Iris Apps",
    );
    settings_page.append(&open_sites_portal_button);
    let update_check_button = action_button(
        "view-refresh-symbolic",
        "Check Updates",
        "Check for updates",
    );
    let update_status = gtk::Label::new(None);
    update_status.add_css_class("iris-muted");
    update_status.set_xalign(0.0);
    update_status.set_wrap(true);
    settings_page.append(&update_auto_check);
    settings_page.append(&update_auto_install);
    settings_page.append(&update_check_button);
    settings_page.append(&update_status);
    let recovery_phrase_button = action_button(
        "dialog-password-symbolic",
        "Recovery phrase",
        "Recovery phrase",
    );
    settings_page.append(&recovery_phrase_button);
    let logout_button = action_button("system-log-out-symbolic", "Log Out", "Log out");
    logout_button.add_css_class("destructive-action");
    settings_page.append(&logout_button);

    stack.add_titled(&dashboard, Some("drive"), "My Drive");
    stack.add_titled(&peers_page, Some("devices"), "Devices");
    stack.add_titled(&shares_page, Some("shares"), "Shares");
    stack.add_titled(&backups_page, Some("backups"), "Backup");
    stack.add_titled(&network_page, Some("network"), "Network");
    stack.add_titled(&settings_page, Some("settings"), "Settings");

    let nav_items = [
        ("drive", "drive-harddisk-symbolic", "My Drive"),
        ("devices", "system-users-symbolic", "Devices"),
        ("shares", "emblem-shared-symbolic", "Shares"),
        ("backups", "security-high-symbolic", "Backup"),
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
    let sidebar_open = action_button("folder-open-symbolic", "Open", "Open drive folder");
    sidebar_open.add_css_class("iris-sidebar-action-button");
    sidebar_open.set_halign(gtk::Align::Fill);
    sidebar.append(&sidebar_open);

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

    let (update_sender, update_receiver) = mpsc::channel();
    let (backup_check_sender, backup_check_receiver) = mpsc::channel();
    let model = Rc::new(AppModel {
        application: app.clone(),
        ui: Ui {
            sidebar,
            update_bar,
            update_label,
            update_auto_check,
            update_auto_install,
            update_install_button,
            update_check_button,
            update_status,
            setup,
            stack,
            sidebar_online,
            main_view,
            main,
            drive_title,
            drive_message,
            status_pill,
            status,
            folder,
            app_key,
            device,
            snapshot,
            files,
            storage,
            devices,
            account_app_key,
            account_device,
            account_authorization,
            approve_box,
            approve_device_button,
            add_recovery_key_button,
            reset_invite_button,
            notice,
            drives,
            peers,
            backups,
            shares,
            fips,
            relays,
            blossom,
            tray_on_close,
            launch_on_startup,
            local_nhash_resolver,
            open_sites_portal_button,
            recovery_phrase_button,
            logout_button,
            relay_entry,
            backup_entry,
            backup_label_entry,
            share_source_entry,
            share_invite_entry,
            create_share_button,
            accept_share_invite_button,
            last_share_invite,
            copy_last_share_invite_button,
            copy_share_identity_button,
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
        setup_recovery_words: RefCell::new(vec![String::new(); RECOVERY_PHRASE_WORD_COUNT]),
        setup_recovery_word_index: Cell::new(0),
        tray: RefCell::new(None),
        tray_available: Cell::new(false),
        settings_refreshing: Cell::new(false),
        update: RefCell::new(updater::UpdateState {
            auto_check: read_auto_check_updates(),
            auto_install: read_auto_install_updates(),
            ..updater::UpdateState::default()
        }),
        update_policy: RefCell::new(UpdateAutoCheckPolicy::new(Duration::from_secs(
            update_poll_interval_secs(),
        ))),
        update_sender,
        update_receiver: RefCell::new(update_receiver),
        backup_check_sender,
        backup_check_receiver: RefCell::new(backup_check_receiver),
        backup_checking: Cell::new(false),
        tray_sync_running: Arc::new(AtomicBool::new(false)),
        closed_to_tray: Cell::new(false),
        launch_on_startup_synced: Cell::new(None),
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
        let button = sidebar_open.clone();
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
        copy_app_key_button
            .connect_clicked(move |_| copy_account_key(&model, "current_app_key_npub"));
    }
    {
        let model = Rc::clone(&model);
        copy_device_button.connect_clicked(move |_| copy_account_key(&model, "device_npub"));
    }
    {
        let button = model.ui.approve_device_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| show_add_device_dialog(&model));
    }
    {
        let button = model.ui.add_recovery_key_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| show_add_recovery_key_dialog(&model));
    }
    {
        let button = model.ui.reset_invite_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| reset_invite(&model));
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
    {
        let button = model.ui.create_share_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| create_share(&model));
    }
    {
        let button = model.ui.accept_share_invite_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| accept_share_invite(&model));
    }
    {
        let button = model.ui.copy_last_share_invite_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| copy_last_share_invite(&model));
    }
    {
        let button = model.ui.copy_share_identity_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| copy_share_identity(&model));
    }
    model.ui.tray_on_close.connect_toggled(|button| {
        write_close_to_tray_on_close(button.is_active());
    });
    {
        let button = model.ui.launch_on_startup.clone();
        let model = Rc::clone(&model);
        button.connect_toggled(move |button| {
            if model.settings_refreshing.get() {
                return;
            }
            set_launch_on_startup(&model, button.is_active());
        });
    }
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
    {
        let button = model.ui.open_sites_portal_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| open_sites_portal(&model));
    }
    {
        let button = model.ui.update_auto_check.clone();
        let model = Rc::clone(&model);
        button.connect_toggled(move |button| {
            if model.settings_refreshing.get() {
                return;
            }
            set_auto_check_updates(&model, button.is_active());
        });
    }
    {
        let button = model.ui.update_auto_install.clone();
        let model = Rc::clone(&model);
        button.connect_toggled(move |button| {
            if model.settings_refreshing.get() {
                return;
            }
            set_auto_install_updates(&model, button.is_active());
        });
    }
    {
        let button = model.ui.update_check_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| check_updates(&model, true));
    }
    {
        let button = model.ui.update_install_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| download_update(&model));
    }
    {
        let button = model.ui.recovery_phrase_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| show_recovery_phrase_dialog(&model));
    }
    {
        let button = model.ui.logout_button.clone();
        let model = Rc::clone(&model);
        button.connect_clicked(move |_| logout(&model));
    }

    connect_tray(&model, &window);

    {
        let model = Rc::clone(&model);
        glib::timeout_add_local(Duration::from_millis(150), move || {
            if model.retired.get() {
                return glib::ControlFlow::Break;
            }
            drain_backup_check_events(&model);
            glib::ControlFlow::Continue
        });
    }

    {
        let model = Rc::clone(&model);
        glib::timeout_add_seconds_local(5, move || {
            if model.retired.get() {
                return glib::ControlFlow::Break;
            }
            if model.ui.main_view.is_visible() {
                refresh(&model);
            }
            drain_update_events(&model);
            check_updates_if_due(&model);
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

    register_active_model(&model);
    render_update_state(&model);
    if present {
        window.present();
    }
    {
        let model = Rc::clone(&model);
        glib::idle_add_local_once(move || {
            refresh(&model);
            check_updates_if_due(&model);
            drain_pending_launch_inputs(&model);
        });
    }
}
