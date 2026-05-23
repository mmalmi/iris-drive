#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn render_setup(model: &AppRef) {
    clear_box(&model.ui.setup);
    match *model.setup_screen.borrow() {
        SetupScreen::Welcome => render_setup_welcome(model),
        SetupScreen::Create => render_create_profile(model),
        SetupScreen::Restore => render_restore_profile(model),
        SetupScreen::Link => render_link_device(model),
    }
}

pub(crate) fn render_awaiting_approval(model: &AppRef, json: &Value, sync_running: bool) {
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
        account.get("device_link_request").unwrap_or(&Value::Null),
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

pub(crate) fn setup_container(model: &AppRef, title: &str) -> gtk::Box {
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

pub(crate) fn render_setup_welcome(model: &AppRef) {
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

pub(crate) fn welcome_button(label: &str, icon_name: &str, primary: bool) -> gtk::Button {
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

pub(crate) fn render_create_profile(model: &AppRef) {
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

pub(crate) fn render_restore_profile(model: &AppRef) {
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

pub(crate) fn render_link_device(model: &AppRef) {
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

pub(crate) fn append_centered_setup(model: &AppRef, child: &gtk::Box) {
    let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top.set_vexpand(true);
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.set_vexpand(true);
    model.ui.setup.append(&top);
    model.ui.setup.append(child);
    model.ui.setup.append(&bottom);
}

pub(crate) fn setup_notice() -> gtk::Label {
    let notice = gtk::Label::new(None);
    notice.add_css_class("iris-notice");
    notice.set_xalign(0.0);
    notice.set_wrap(true);
    notice
}

pub(crate) fn initialize_drive(model: &AppRef) {
    match create_profile("") {
        Ok(()) => {
            model.ui.notice.set_text("Initialized");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn create_profile(label: &str) -> Result<(), String> {
    let mut init_args = vec!["init".to_string(), "--force".to_string()];
    let label = label.trim();
    if !label.is_empty() {
        init_args.push("--label".to_string());
        init_args.push(label.to_string());
    }

    run_idrive_owned(&init_args)
}

pub(crate) fn restore_profile(secret: &str, label: &str) -> Result<(), String> {
    let mut args = vec!["restore".to_string(), secret.to_string()];
    let label = label.trim();
    if !label.is_empty() {
        args.push("--label".to_string());
        args.push(label.to_string());
    }
    run_idrive_owned(&args)
}

pub(crate) fn link_device(owner: &str, label: &str) -> Result<(), String> {
    let mut args = vec!["link".to_string(), owner.to_string()];
    let label = label.trim();
    if !label.is_empty() {
        args.push("--label".to_string());
        args.push(label.to_string());
    }
    run_idrive_owned(&args)
}

pub(crate) fn revoke_device(device: &str) -> Result<(), String> {
    run_idrive(["revoke", device])
}
